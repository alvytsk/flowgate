# Flowgate

A Rust web framework for embedded Linux systems with FastAPI-inspired ergonomics.

Built on [hyper 1.x](https://github.com/hyperium/hyper) and [tokio](https://tokio.rs), Flowgate targets constrained environments where memory footprint and compile-time safety matter. Single-crate, single-threaded by default, zero proc macros.

## Quick Start

Add Flowgate to your `Cargo.toml`:

```toml
[dependencies]
flowgate = { path = "." }    # or from your registry
serde = { version = "1.0", features = ["derive"] }
```

A minimal server:

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
async fn main() {
    let app = App::new().get("/hello", hello);
    flowgate::server::serve(app, ServerConfig::new()).await.unwrap();
}
```

## Features

- **Type-safe extractors** -- handler arguments are extracted from the request at compile time. The last argument may consume the body (`FromRequest`); all preceding arguments use headers only (`FromRequestParts`).
- **Shared application state** -- `App::with_state(S)` wraps state in `Arc` and makes it available to every handler via `State<T>`. Sub-state projection through `FromRef` keeps extraction cheap.
- **Middleware chain** -- implement `Middleware<S>` to intercept requests. The chain is fully `Arc`-based for cheap cloning across connections.
- **Configurable limits** -- body size, keep-alive, header read timeout, max headers -- all exposed through `ServerConfig` with embedded-safe defaults.
- **matchit router** -- zero-allocation radix trie with path parameters (`:id`, `*rest`).

## Application State

```rust
use std::sync::Arc;
use flowgate::{App, ServerConfig};
use flowgate::extract::state::State;
use flowgate::extract::FromRef;

#[derive(Clone)]
struct AppState {
    db: Arc<Db>,
}

struct Db { /* ... */ }

impl FromRef<AppState> for Arc<Db> {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

async fn handler(State(db): State<Arc<Db>>) -> &'static str {
    "ok"
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let state = AppState { db: Arc::new(Db {}) };
    let app = App::with_state(state).get("/", handler);
    flowgate::server::serve(app, ServerConfig::new()).await.unwrap();
}
```

`State<T>` implements both `FromRequest` and `FromRequestParts`, so it can appear in any handler argument position.

## Middleware

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

Register with `.layer(Timing)` on the `App` builder. A built-in `TracingMiddleware` is included for structured request logging.

## Server Configuration

```rust
use std::time::Duration;
use flowgate::ServerConfig;

let config = ServerConfig::new()
    .addr("0.0.0.0:3000")
    .json_body_limit(128 * 1024)     // 128 KiB
    .keep_alive(true)
    .header_read_timeout(Some(Duration::from_secs(5)))
    .max_headers(Some(32));
```

| Option | Default | Notes |
|--------|---------|-------|
| `addr` | `0.0.0.0:8080` | Bind address |
| `json_body_limit` | 256 KiB | Max JSON body size (413 on exceed) |
| `keep_alive` | `true` | HTTP/1.1 keep-alive |
| `header_read_timeout` | 5 s | `None` to disable |
| `max_headers` | 64 | `None` for hyper default |
| `enable_default_tracing` | `true` | Auto-init `tracing-subscriber` |

## Feature Flags

| Flag | Default | Description |
|------|---------|-------------|
| `tracing-fmt` | yes | Sets up `tracing-subscriber` with env-filter |
| `multi-thread` | no | Enables tokio multi-threaded runtime |
| `ws` | no | WebSocket support (v0.2) |
| `tls` | no | TLS via rustls (v0.2) |

## Building and Testing

```bash
cargo build                        # Build the library
cargo build --all-features         # Build with all feature flags
cargo test                         # Run all integration tests
cargo clippy --all-targets         # Lint (zero warnings required)
cargo run --example hello          # Run the demo server on :8080
```

## Project Structure

```
src/
  lib.rs          Public API re-exports
  app.rs          App builder (state, routes, middleware)
  server.rs       TCP accept loop, hyper wiring
  router.rs       matchit radix trie, route matching
  handler.rs      Handler trait + macro-generated impls (0-8 args)
  middleware.rs    Middleware trait, Next chain, TracingMiddleware
  config.rs       ServerConfig with embedded-safe defaults
  body.rs         Request/Response type aliases
  context.rs      RequestContext, RouteParams (injected per-request)
  error.rs        Rejection types implementing IntoResponse
  response.rs     IntoResponse trait + impls
  extract/
    mod.rs        FromRequest, FromRequestParts, FromRef traits
    json.rs       Json<T> extractor/responder
    state.rs      State<T> extractor
examples/
  hello.rs        Full demo with state, sub-state, middleware
tests/
  integration.rs  Round-trip HTTP tests
docs/
  architecture.md Detailed architecture document
```

## Documentation

- **[Architecture](docs/architecture.md)** -- layer diagram, handler erasure, ownership model, and design rationale
- **`cargo doc --no-deps --open`** -- API reference from doc comments

## Minimum Supported Rust Version

Rust **1.75** (edition 2021).

## License

MIT
