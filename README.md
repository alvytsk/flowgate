# ⚙️ Flowgate

**Embedded-First • Zero Proc Macros • Compile-Time Safety**

[![License](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75+-DEA584?logo=rust)](https://www.rust-lang.org)

*Flowgate is a web framework for embedded Linux, built for developers who need real-time control without runtime surprises. Powered by hyper 1.x and tokio, it delivers FastAPI-inspired ergonomics in a single-crate, single-threaded package with zero proc macros.*

---

[Features](#-key-features) • [Quick Start](#-quick-start) • [Core Concepts](#-core-concepts) • [Configuration](#-server-configuration) • [Architecture](docs/architecture.md)

---

## 💎 Why Flowgate?

In an ecosystem of heavyweight async frameworks, Flowgate is purpose-built for **constrained environments** where resources are tight.

| Feature | Flowgate | Axum | Actix-web | Rocket |
| :--- | :---: | :---: | :---: | :---: |
| **Single-Crate Design** | ✅ | ❌ | ❌ | ❌ |
| **Zero Proc Macros** | ✅ | ✅ | ❌ | ❌ |
| **Single-Threaded Default** | ✅ | ❌ | ❌ | ❌ |
| **Resource-Constrained Defaults** | ✅ | ❌ | ❌ | ❌ |
| **Sub-State Projection** | ✅ | ✅ | ❌ | ❌ |
| **Direct hyper 1.x (no Tower)** | ✅ | ❌ | ❌ | ❌ |

> **Route handlers are plain async functions with typed arguments — inspired by FastAPI's simplicity, powered by Rust's type system.**

---

## ✨ Key Features

- **⚙️ Embedded-First**: Single-threaded tokio runtime by default with configurable body limits, header caps, and keep-alive — tuned for resource-constrained Linux systems.
- **🔒 Compile-Time Safety**: Handler arguments are validated at compile time via `FromRequest` / `FromRequestParts` traits. No runtime reflection, no proc macros.
- **🧩 Handler Erasure**: Macro-generated impls for 0–8 extractor arguments bridge type-safe handlers to an object-safe `Endpoint` trait — clean generics, zero proc macros.
- **📦 Single Crate**: One dependency in your `Cargo.toml`. No workspace sprawl, no adapter crates, no version matrix.
- **🔗 Arc-Based Middleware**: Fully owned middleware chain with `Arc<dyn Middleware<S>>`. Cheap cloning across connections, uniform ownership throughout.
- **🌲 Zero-Allocation Router**: Built on [matchit](https://github.com/ibraheemdev/matchit) — a radix trie with path parameters (`:id`, `*rest`) and zero heap allocations on match.

---

## 🏁 Quick Start

### Installation

```toml
[dependencies]
flowgate = "0.1"
serde = { version = "1.0", features = ["derive"] }
```

### The "Hello, Flowgate" Example

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
    let app = App::new().get("/hello", hello);
    flowgate::server::serve(app, ServerConfig::from_env()).await?;
    Ok(())
}
```

```bash
cargo run --example hello
# GET http://localhost:8080/hello → {"msg":"hello world"}
```

---

## 🛠 Core Concepts

### 🧬 Type-Safe Extractors

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
    State(db): State<Arc<Db>>,   // FromRequestParts — headers only
    Json(body): Json<CreateUser>, // FromRequest — consumes body (must be last)
) -> Json<User> {
    Json(User { id: 1, name: body.name })
}
```

### 📡 Application State & Sub-State Projection

Wrap shared state in `Arc` once. Extract fine-grained sub-state in handlers via `FromRef` — no cloning the entire state tree.

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

### 🧱 Middleware

Implement the `Middleware<S>` trait to intercept requests. The chain is fully `Arc`-based — cheap to clone across connections.

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

### 🛤 Routing

```rust
let app = App::with_state(state)
    .layer(TracingMiddleware)
    .get("/health", health)
    .post("/users", create_user)
    .get("/users/:id", get_user)
    .get("/files/*path", serve_file);
```

Path parameters (`:id`) and catch-all segments (`*path`) are powered by the matchit radix trie.

---

## ⚡ Server Configuration

Read from environment or configure explicitly — embedded-safe defaults out of the box.

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
    .keep_alive(true)
    .header_read_timeout(Some(Duration::from_secs(5)))
    .max_headers(Some(32));
```

| Option | Default | Notes |
| :--- | :---: | :--- |
| `host` | `0.0.0.0` | Bind address |
| `port` | `8080` | Bind port |
| `json_body_limit` | 256 KiB | Max JSON body size (413 on exceed) |
| `keep_alive` | `true` | HTTP/1.1 keep-alive |
| `header_read_timeout` | 5 s | `None` to disable |
| `max_headers` | 64 | `None` for hyper default |
| `enable_default_tracing` | `true` | Auto-init `tracing-subscriber` |

**Environment variables** (`HOST`, `PORT`) override defaults when using `ServerConfig::from_env()`.

---

## 🚩 Feature Flags

| Flag | Default | Description |
| :--- | :---: | :--- |
| `tracing-fmt` | ✅ | Sets up `tracing-subscriber` with env-filter |
| `multi-thread` | — | Enables tokio multi-threaded runtime |
| `ws` | — | WebSocket support (v0.2) |
| `tls` | — | TLS via rustls (v0.2) |

```bash
cargo build --all-features         # Build with ws, tls, multi-thread
```

---

## 🏗 Building & Testing

```bash
cargo build                        # Build the library
cargo test                         # Run all integration tests (22 tests)
cargo clippy --all-targets         # Lint (zero warnings required)
cargo doc --no-deps --open         # Browse API docs
cargo run --example hello          # Run demo server on :8080
PORT=3000 cargo run --example hello  # Override port via env
```

---

## 📂 Project Structure

```
src/
  lib.rs          Public API re-exports
  app.rs          App builder (state, routes, middleware)
  server.rs       TCP accept loop, hyper wiring
  router.rs       matchit radix trie, route matching
  handler.rs      Handler trait + macro-generated impls (0–8 args)
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
  architecture.md Layer diagram, handler erasure, ownership model
```

---

## 📖 Documentation

- **[Architecture](docs/architecture.md)** — layer diagram, handler erasure, ownership model, and design rationale
- **`cargo doc --no-deps --open`** — API reference from doc comments

---

## 📋 Minimum Supported Rust Version

Rust **1.75** (edition 2021).

---

## 📄 License

Flowgate is released under the **MIT License**. See [LICENSE](./LICENSE) for details.
