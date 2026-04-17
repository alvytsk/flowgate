# Flowgate

**Embedded-First / Zero Proc Macros / Compile-Time Safety**

[![License](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75+-DEA584?logo=rust)](https://www.rust-lang.org)

*Flowgate is a web framework for embedded Linux, built for developers who need real-time control without runtime surprises. Powered by hyper 1.x and tokio, it delivers FastAPI-inspired ergonomics in a single-crate, single-threaded package with zero proc macros.*

---

[Features](#key-features) | [Quick Start](#quick-start) | [Route Groups](#route-groups) | [Middleware](#middleware) | [Observability](#observability) | [OpenAPI](#openapi-docs) | [Configuration](#server-configuration) | [Architecture](docs/architecture.md)

---

## Why Flowgate?

In an ecosystem of heavyweight async frameworks, Flowgate is purpose-built for **constrained environments** where resources are tight.

| Feature | Flowgate | Axum | Actix-web | Rocket |
| :--- | :---: | :---: | :---: | :---: |
| **Single-Crate Design** | yes | no | no | no |
| **Zero Proc Macros** | yes | yes | no | no |
| **Single-Threaded Default** | yes | no | no | no |
| **Resource-Constrained Defaults** | yes | no | no | no |
| **Route Groups with Inheritance** | yes | via `nest` | scoped | no |
| **Built-in OpenAPI + Docs UI** | yes | via utoipa | no | no |
| **Pre-routing Middleware** | yes | via layers | yes | no |
| **Direct hyper 1.x (no Tower)** | yes | no | no | no |

> **Route handlers are plain async functions with typed arguments -- inspired by FastAPI's simplicity, powered by Rust's type system.**

---

## Key Features

- **Embedded-First**: Single-threaded tokio runtime by default with configurable body limits, header caps, and keep-alive -- tuned for resource-constrained Linux systems.
- **Compile-Time Safety**: Handler arguments are validated at compile time via `FromRequest` / `FromRequestParts` traits. No runtime reflection, no proc macros.
- **Route Groups**: Nested groups with prefix, middleware, and tag inheritance. Flattened at finalization -- zero runtime overhead.
- **Pre- and Post-routing Middleware**: `PreMiddleware` runs before route matching (request IDs, auth shortcuts). `Middleware` runs after (tracing, timeouts, panic recovery).
- **Built-in Operational Middleware**: `RequestIdMiddleware`, `TimeoutMiddleware`, `RecoverMiddleware` (feature-gated) -- production-ready out of the box.
- **OpenAPI + Docs UI**: Feature-gated spec generation and Scalar docs UI at `/docs`. Manual operation metadata, no proc-macro overhead.
- **Order-Insensitive Builder**: Routes, groups, and middleware can be added in any order. `finalize()` merges everything correctly at serve time.
- **Single Crate**: One dependency in your `Cargo.toml`. No workspace sprawl, no adapter crates, no version matrix.
- **Zero-Allocation Router**: Built on [matchit](https://github.com/ibraheemdev/matchit) -- a radix trie with path parameters and zero heap allocations on match.

---

## Quick Start

### Installation

```toml
[dependencies]
flowgate = "0.2"
serde = { version = "1.0", features = ["derive"] }
```

### Minimal Example

```rust
use flowgate::{App, ServerConfig};
use flowgate::extract::json::Json;
use serde::Serialize;

#[derive(Serialize)]
struct Hello { msg: String }

async fn hello() -> Json<Hello> {
    Json(Hello { msg: "hello world".into() })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::new().get("/hello", hello)?;
    flowgate::server::serve(app, ServerConfig::from_env()).await?;
    Ok(())
}
```

```bash
cargo run --example hello
# GET http://localhost:8080/hello -> {"msg":"hello world"}

cargo run --example groups --features openapi
# Routes: /health, /api/v1/users/{id}, /api/v1/admin/stats
# Docs:   http://localhost:8080/docs
```

---

## Core Concepts

### Type-Safe Extractors

Handler arguments are extracted from the request automatically. The **last** argument may consume the body (`FromRequest`); all preceding arguments use headers only (`FromRequestParts`).

```rust
use flowgate::extract::json::Json;
use flowgate::extract::state::State;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
struct CreateUser { name: String }

#[derive(Serialize)]
struct User { id: u64, name: String }

async fn create_user(
    State(db): State<Arc<Db>>,   // FromRequestParts -- headers only
    Json(body): Json<CreateUser>, // FromRequest -- consumes body (must be last)
) -> Json<User> {
    Json(User { id: 1, name: body.name })
}
```

### Application State & Sub-State Projection

Wrap shared state in `Arc` once. Extract fine-grained sub-state in handlers via `FromRef` -- no cloning the entire state tree.

```rust
use flowgate::{App, ServerConfig};
use flowgate::extract::state::State;
use flowgate::extract::FromRef;
use std::sync::Arc;

#[derive(Clone)]
struct AppState {
    db: Arc<Db>,
    app_name: String,
}

impl FromRef<AppState> for Arc<Db> {
    fn from_ref(state: &AppState) -> Self { state.db.clone() }
}

async fn health(
    State(db): State<Arc<Db>>,
    State(name): State<String>,
) -> &'static str {
    "ok"
}
```

---

## Route Groups

Groups carry a path prefix, middleware, and tags that are inherited by all routes and subgroups. Groups are flattened at finalization -- zero runtime tree walking.

```rust
use std::time::Duration;
use flowgate::{App, AppMeta, Group, RequestIdMiddleware, TimeoutMiddleware};
use flowgate::middleware::TracingMiddleware;

let app = App::with_state(state)
    .meta(AppMeta::new("My API", "1.0.0"))
    .pre(RequestIdMiddleware)
    .get("/health", health)?
    .group(
        Group::new("/api/v1")
            .tag("api")
            .layer(TimeoutMiddleware::new(Duration::from_secs(30)))
            .get("/users/{id}", get_user)
            .post("/users", create_user)
            .group(
                Group::new("/admin")
                    .tag("admin")
                    .get("/stats", admin_stats)
            )
    )
    .layer(TracingMiddleware);  // order doesn't matter

// Routes registered:
//   GET  /health
//   GET  /api/v1/users/{id}       (api tag, 30s timeout)
//   POST /api/v1/users            (api tag, 30s timeout)
//   GET  /api/v1/admin/stats      (api + admin tags, 30s timeout)
```

Middleware added with `.layer()` on a Group applies only to routes within that group (and its subgroups). App-level `.layer()` applies to all routes.

---

## Middleware

### Post-routing Middleware

Runs after route matching. Has access to route params via `RequestContext`.

```rust
use std::sync::Arc;
use flowgate::body::{Request, Response};
use flowgate::middleware::{Middleware, Next};
use flowgate::handler::BoxFuture;

struct Timing;

impl<S: Send + Sync + 'static> Middleware<S> for Timing {
    fn call(&self, req: Request, state: Arc<S>, next: Next<S>) -> BoxFuture {
        Box::pin(async move {
            let start = std::time::Instant::now();
            let resp = next.run(req, state).await;
            println!("took {:?}", start.elapsed());
            resp
        })
    }
}
```

### Pre-routing Middleware

Runs before route matching. Useful for request IDs, auth shortcuts, and path normalization.

```rust
use flowgate::RequestIdMiddleware;

let app = App::new()
    .pre(RequestIdMiddleware)  // generates/propagates X-Request-Id
    .get("/health", health)?;
```

### Built-in Middleware

| Middleware | Type | Description |
|:---|:---|:---|
| `TracingMiddleware` | Post-routing | Logs method, path, status, duration |
| `RequestIdMiddleware` | Pre-routing | Generates/propagates `X-Request-Id` header |
| `TimeoutMiddleware` | Post-routing | Returns 504 if handler exceeds duration |
| `RecoverMiddleware` | Post-routing | Catches handler panics, returns 500 (requires `recover` feature) |

---

## Observability

Register a `MetricsObserver` to receive a `RequestEvent` for every request — matched routes, 404s, and 405s alike. Observers are the hook point for Prometheus, StatsD, OpenTelemetry, or any custom telemetry pipeline.

```rust
use flowgate::{App, MetricsObserver, RequestEvent};

struct Counter;

impl MetricsObserver for Counter {
    fn on_request(&self, event: &RequestEvent<'_>) {
        // `event.route_pattern` is the *registered* pattern, e.g. `/users/{id}`,
        // not the per-request path `/users/42`. Use it directly as a label.
        let label = event.route_pattern.unwrap_or("<unmatched>");
        tracing::info!(
            method = %event.method,
            route = label,
            status = event.status.as_u16(),
            duration_us = event.duration.as_micros() as u64,
            "request"
        );
    }
}

let app = App::new()
    .get("/users/{id}", get_user)?
    .observe(Counter);
```

### Designed for real telemetry backends

`RequestEvent` is deliberately keyed on the **matched route pattern**, not the raw request path. This matters:

- A raw path like `/users/42` produces unbounded label cardinality — Prometheus, StatsD, and every time-series backend will eventually degrade (or cost you real money) if you feed them per-user paths as labels.
- The registered pattern `/users/{id}` is bounded by your route table. It is the *correct* label for HTTP metrics, matching the convention used by every mature telemetry system.

404 and 405 responses emit the event with `route_pattern: None` — bucket them under a sentinel label of your choice in the observer implementation.

### Callback contract

- **Synchronous.** `on_request` runs inline on the dispatch path. Keep it cheap: an atomic increment, a metric update, a channel send.
- **Hand off anything that blocks or does I/O.** Pushing to a remote backend, writing to a file, or serializing large payloads — send the event (or a small derived struct) over an `mpsc` channel to a background task. Do not `await` or perform I/O in the observer itself.
- **Multiple observers supported.** Each call to `.observe(...)` registers an additional observer; they fire in registration order.
- **Zero cost when unused.** If no observer is registered, the dispatch path skips the wall-clock read entirely and allocates nothing for observation.

---

## OpenAPI Docs

Enable the `openapi` feature to get automatic spec generation and a Scalar docs UI.

```toml
[dependencies]
flowgate = { version = "0.2", features = ["openapi"] }
```

```rust
use flowgate::{App, AppMeta, OperationMeta};

let app = App::new()
    .meta(AppMeta::new("My API", "1.0.0"))
    .get_with(
        "/users/{id}",
        get_user,
        OperationMeta::new()
            .summary("Get user by ID")
            .tag("users")
            .response(200, "User found")
            .response(404, "Not found"),
    )?
    .with_openapi();

// Serves:
//   GET /openapi.json  -- OpenAPI 3.1 spec
//   GET /docs           -- Scalar API reference UI
```

When the `openapi` feature is disabled, `OperationMeta` becomes a zero-size stub with no-op builders -- your code compiles identically.

---

## Server Configuration

Read from environment or configure explicitly -- embedded-safe defaults out of the box.

```rust
use std::time::Duration;
use flowgate::ServerConfig;

// From environment (reads HOST, PORT):
let config = ServerConfig::from_env();

// Or explicit:
let config = ServerConfig::new()
    .host("127.0.0.1")
    .port(3000)
    .json_body_limit(128 * 1024)     // 128 KiB
    .body_read_timeout(Some(Duration::from_secs(30)))
    .keep_alive(true)
    .header_read_timeout(Some(Duration::from_secs(5)))
    .max_headers(Some(32));
```

| Option | Default | Notes |
| :--- | :---: | :--- |
| `host` | `0.0.0.0` | Bind address |
| `port` | `8080` | Bind port |
| `json_body_limit` | 256 KiB | Max JSON body size (413 on exceed) |
| `body_read_timeout` | 30 s | Bounds `Json<T>` body collect — returns 408 + `Connection: close` on stall. `None` to disable. |
| `keep_alive` | `true` | HTTP/1.1 keep-alive |
| `header_read_timeout` | 5 s | `None` to disable |
| `max_headers` | 64 | `None` for hyper default |
| `enable_default_tracing` | `true` | Auto-init `tracing-subscriber` |

**Environment variables** (`HOST`, `PORT`) override defaults when using `ServerConfig::from_env()`.

---

## Feature Flags

| Flag | Default | Description |
| :--- | :---: | :--- |
| `tracing-fmt` | yes | Sets up `tracing-subscriber` with env-filter |
| `openapi` | -- | OpenAPI 3.1 spec generation + Scalar docs UI |
| `recover` | -- | `RecoverMiddleware` for panic recovery |
| `multi-thread` | -- | Enables tokio multi-threaded runtime |
| `ws` | -- | WebSocket support (planned) |
| `tls` | -- | TLS via rustls (planned) |

```bash
cargo build --all-features
```

---

## Building & Testing

```bash
cargo build                           # Build the library
cargo test                            # Run all tests
cargo test --features openapi         # Include OpenAPI tests
cargo clippy --all-targets            # Lint (zero warnings required)
cargo doc --no-deps --open            # Browse API docs
cargo run --example hello             # Run minimal demo on :8080
cargo run --example groups --features openapi  # Groups demo with OpenAPI docs
```

---

## Project Structure

```
src/
  lib.rs              Public API re-exports
  app.rs              App builder, RawRoute, finalize(), AppMeta
  server.rs           TCP accept loop, hyper wiring, startup banner
  router.rs           matchit radix trie, CompiledRoute
  handler.rs          Handler trait + macro-generated impls (0-8 args)
  group.rs            Route group builder, flatten, path normalization
  config.rs           ServerConfig with embedded-safe defaults
  body.rs             Request/Response type aliases
  context.rs          RequestContext, RouteParams (injected per-request)
  error.rs            Rejection types implementing IntoResponse
  observer.rs         MetricsObserver trait + RequestEvent
  response.rs         IntoResponse trait + impls
  extract/
    mod.rs            FromRequest, FromRequestParts, FromRef traits
    json.rs           Json<T> extractor/responder
    path.rs           Path<T> extractor (single, tuple, struct)
    query.rs          Query<T> extractor
    state.rs          State<T> extractor
    request_id.rs     RequestId extractor
  middleware/
    mod.rs            Middleware, PreMiddleware, Next, PreNext, TracingMiddleware
    request_id.rs     RequestIdMiddleware
    timeout.rs        TimeoutMiddleware
    recover.rs        RecoverMiddleware (feature-gated)
  openapi/            (feature-gated)
    mod.rs            Module re-exports
    meta.rs           OperationMeta, ParamMeta, SchemaObject
    spec.rs           OpenAPI 3.1 spec generation
    ui.rs             Scalar docs UI HTML
  openapi_stub.rs     Zero-size OperationMeta stub (when openapi off)
examples/
  hello.rs            Minimal demo with state and middleware
  groups.rs           Route groups, request IDs, nested middleware (requires openapi feature)
tests/
  integration.rs      Round-trip HTTP tests
docs/
  architecture.md     Layer diagram, builder/runtime split, design decisions
```

---

## Documentation

- **[Architecture](docs/architecture.md)** -- layer diagram, builder/runtime split, ownership model, and design rationale
- **[Performance baseline](docs/perf-baseline.md)** -- benchmark setup, caveats, and the reference matrix future changes compare against
- **[Changelog](CHANGELOG.md)** -- release notes
- **`cargo doc --no-deps --open`** -- API reference from doc comments

---

## Minimum Supported Rust Version

Rust **1.75** (edition 2021).

---

## License

Flowgate is released under the **MIT License**. See [LICENSE](./LICENSE) for details.
