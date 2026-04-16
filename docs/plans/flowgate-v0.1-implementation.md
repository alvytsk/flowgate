# Flowgate v0.1 Implementation Plan

## Context

Flowgate is a new Rust web framework for embedded Linux systems with FastAPI-inspired ergonomics. The repository is completely empty (no files, no commits). This plan covers the v0.1 foundation: a single-crate project with clean module separation, working HTTP server, JSON extractors, shared state, middleware, and a runnable example.

The goal is not a full web framework but a believable, compilable foundation that demonstrates the layered architecture and intended developer experience.

## Key Architectural Decisions

| Decision | Choice | Why |
|---|---|---|
| Crate structure | Single crate, module boundaries | Premature crate splits are tax; modules can be promoted to crates in v0.2 |
| HTTP foundation | hyper 1.x + hyper-util | Mature, stable 1.x API; we own the server loop, hyper handles protocol |
| Routing | matchit | Zero-allocation radix trie, tiny footprint, perfect for embedded |
| Request body | `hyper::body::Incoming` | Hyper's concrete receive-stream type for server requests |
| Response body | `Full<Bytes>` | Concrete "whole body in memory" type; streaming deferred to v0.2 |
| Middleware | Own simple trait (not tower), fully owned chain | No `poll_ready`; `Arc`-based chain avoids lifetime issues with boxed futures |
| Runtime default | Single-threaded tokio | Embedded-first; `multi-thread` feature flag. Library does not own runtime — users control runtime flavor. |
| Handler adapters | `macro_rules!` for tuple extractors | Avoids proc macros; proven pattern (axum uses the same approach) |
| Extractor traits | `FromRequest` / `FromRequestParts` | Split allows body-consuming extractors (Json) vs header-only (State) |
| Handler erasure | Two-layer: generic `Handler<T,S>` + object-safe `Endpoint<S>` | User-facing adapters are generic; router stores `Arc<dyn Endpoint<S>>` via boxed futures |
| Ownership model | `Arc<S>` everywhere in erased/middleware layer | Endpoint and Middleware both take `Arc<S>` — uniform ownership, no `&S` vs `Arc<S>` mismatch |
| Framework context | `RequestContext` in extensions before dispatch | Single channel for runtime config (body limit) + route params; extractors read from extensions |
| Sub-state extraction | `FromRef<S>` projection trait | Avoids forcing Clone on entire state; enables `State<Arc<Db>>` style extraction |
| Route params | Owned `RouteParams` stored in request extensions at match time | matchit returns borrowed `Params<'k,'v>`; must copy to owned type before storing |
| JSON body limit | Enforced in v0.1 via `http_body_util::Limited` | Unbounded body reads are unsafe for embedded; default 256 KiB |
| HTTP/1 config | Flowgate owns explicit knobs, not hyper defaults | hyper docs say defaults are not stable; timeouts require `Builder::timer(TokioTimer)` |

## Dependencies

**Always-on:** tokio (rt, net, macros), hyper 1.x, hyper-util (incl. `TokioTimer` for timeout support), http 1.x, http-body 1.x, http-body-util, bytes, serde + serde_json, tracing, matchit

**Feature-gated:** tracing-subscriber (default on), tokio-tungstenite (ws), tokio-rustls (tls), tokio/rt-multi-thread (multi-thread)

## Module Layout

```
flowgate/
  Cargo.toml
  src/
    lib.rs              # Public facade, re-exports
    app.rs              # App builder, state management (owns Arc<S>)
    router.rs           # Router, route registration, method dispatch, owned RouteParams into extensions
    handler.rs          # Handler<T,S> trait, Endpoint<S> object-safe trait, macro-generated adapters
    extract/
      mod.rs            # FromRequest, FromRequestParts, FromRef traits
      json.rs           # Json<T> extractor + responder (with body size limit via Limited)
      state.rs          # State<T> extractor via FromRef<S> projection
    response.rs         # IntoResponse trait, implementations
    error.rs            # Framework error types, rejection types
    middleware.rs        # Middleware trait (Arc-based chain), TracingMiddleware
    server.rs           # Server::bind().serve(), hyper accept loop, explicit HTTP/1 config + TokioTimer
    body.rs             # RequestBody = Incoming, ResponseBody = Full<Bytes> type aliases
    context.rs          # RequestContext (route params + body limit), inserted into extensions before dispatch
    config.rs           # ServerConfig with embedded-safe defaults (incl. HTTP/1 knobs, body limit)
  examples/
    hello.rs            # Minimal API example with state, JSON, routing
  docs/
    architecture.md     # Architecture documentation
```

**Layer mapping:**

| Conceptual Layer | Module(s) |
|---|---|
| Transport | `server.rs` |
| Protocol | `server.rs` + `body.rs` (hyper handles HTTP) |
| Routing | `router.rs` |
| Extraction/Response | `extract/*`, `response.rs` |
| Middleware | `middleware.rs` |
| Application | `app.rs`, `config.rs` |
| Infrastructure | `error.rs` |

## Core Type Design

### Body types: split request and response

```rust
// src/body.rs
use bytes::Bytes;
use http_body_util::Full;

/// Incoming request body — hyper's streaming receive type.
pub type RequestBody = hyper::body::Incoming;

/// Outgoing response body — whole body in memory.
pub type ResponseBody = Full<Bytes>;

// Framework-level type aliases
pub type Request = http::Request<RequestBody>;
pub type Response = http::Response<ResponseBody>;
```

Hyper delivers request bodies as `Incoming` (a streaming type). Responses use `Full<Bytes>` (buffered). These are distinct types and must not be aliased together.

### Handler erasure: two-layer design

**User-facing** — generic, for type inference on extractor tuples:
```rust
pub trait Handler<T, S>: Clone + Send + 'static {
    fn call(self, req: Request, state: Arc<S>) -> impl Future<Output = Response> + Send;
}
```

**Internal** — object-safe, stored in router. Takes `Arc<S>` (not `&S`) so the returned `BoxFuture` is `'static` without lifetime issues:
```rust
type BoxFuture = Pin<Box<dyn Future<Output = Response> + Send + 'static>>;

pub(crate) trait Endpoint<S>: Send + Sync + 'static {
    fn call(&self, req: Request, state: Arc<S>) -> BoxFuture;
}
```

The bridge struct converts `Handler<T, S>` into `Arc<dyn Endpoint<S>>`. Inside the bridge's `call`, it does `self.handler.clone().call(req, state)` — the handler clones itself (it's a function pointer, zero cost) and the `async move` block owns the `Arc<S>`.

`macro_rules!` generates `Handler` impls for functions with 0..8 extractor arguments. Each generated impl dereferences `Arc<S>` to `&S` for extractor calls:
```rust
// Inside generated Handler impl:
async fn call(self, req: Request, state: Arc<S>) -> Response {
    let (mut parts, body) = req.into_parts();
    let t1 = T1::from_request_parts(&mut parts, &*state).await?;
    // ... remaining extractors
    self(t1, ...).await.into_response()
}
```

### State: `Arc<S>` throughout erased layer, `&S` for extractors

```rust
pub struct App<S = ()> {
    state: Arc<S>,
    router: Router<S>,
    middleware: Arc<[Arc<dyn Middleware<S>>]>,  // Arc slice for cheap cloning
}
```

**Ownership boundary:**
- Erased layer (Endpoint, Middleware, Next) passes `Arc<S>` — required for `'static` boxed futures
- Extractors receive `&S` — the Handler impl dereferences `&*state` before calling extractors
- `FromRequest::from_request(req, state: &S)`
- `FromRequestParts::from_request_parts(parts, state: &S)`

**Sub-state extraction via `FromRef<S>`:**

```rust
// src/extract/mod.rs
pub trait FromRef<S> {
    fn from_ref(state: &S) -> Self;
}

// Identity impl: FromRef<S> for S where S: Clone
impl<S: Clone> FromRef<S> for S {
    fn from_ref(state: &S) -> Self {
        state.clone()
    }
}
```

`State<T>` extracts `T` from `&S` via `FromRef`:

```rust
impl<S, T> FromRequestParts<S> for State<T>
where
    T: FromRef<S>,
    S: Send + Sync,
{
    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(State(T::from_ref(state)))
    }
}
```

This means:
- `State<AppState>` works when `AppState: Clone` (whole-state extraction via identity impl)
- `State<Arc<Db>>` works when user implements `FromRef<AppState> for Arc<Db>` (sub-state projection, cheap Arc clone)
- No Clone pressure on the entire state type for sub-state extraction

### RequestContext: unified framework metadata in extensions

Before dispatch, the server inserts a `RequestContext` into request extensions. This is the single channel for all framework metadata that extractors need at runtime:

```rust
// src/context.rs
#[derive(Clone, Debug)]
pub struct RequestContext {
    /// Owned route params from matchit (copied from borrowed Params<'k,'v>)
    pub route_params: RouteParams,
    /// Body size limit for this request (from ServerConfig)
    pub body_limit: usize,
}

#[derive(Clone, Debug, Default)]
pub struct RouteParams(pub Vec<(String, String)>);
```

**Why a unified context instead of separate extension entries:**
- Extractors like `Json<T>` need the runtime `body_limit` — `FromRequest` only receives `(req, &S)`, not `ServerConfig`
- `Path<T>` in v0.2 needs route params — same channel, no new extension type
- One consistent "framework metadata" source avoids sprinkling special cases

At match time in the router:
```rust
let matchit_match = method_router.at(path)?;
let route_params = RouteParams(
    matchit_match.params.iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
);
req.extensions_mut().insert(RequestContext {
    route_params,
    body_limit: config_body_limit,  // passed from ServerConfig at startup
});
```

### Extractor traits

```rust
pub trait FromRequestParts<S>: Sized {
    type Rejection: IntoResponse;
    fn from_request_parts(parts: &mut Parts, state: &S) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}
pub trait FromRequest<S>: Sized {
    type Rejection: IntoResponse;
    fn from_request(req: Request, state: &S) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}
```

### Json extractor: with body size limit from RequestContext

The extractor reads the body limit from `RequestContext` in request extensions (inserted by the router/server before dispatch), not from a compile-time constant:

```rust
impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for Json<T> {
    async fn from_request(req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let (parts, body) = req.into_parts();
        
        // Read runtime body limit from framework context
        let limit = parts.extensions
            .get::<RequestContext>()
            .map(|ctx| ctx.body_limit)
            .unwrap_or(DEFAULT_BODY_LIMIT);
        
        let limited = Limited::new(body, limit);
        let collected = limited.collect().await.map_err(|e| {
            // http_body_util::Limited wraps errors in a boxed Error.
            // LengthLimitError = body exceeded limit -> 413
            // Other errors = underlying body read failure -> 400
            if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() {
                JsonRejection::PayloadTooLarge
            } else {
                JsonRejection::BodyReadError(e.to_string())
            }
        })?;
        let bytes = collected.to_bytes();
        let value = serde_json::from_slice(&bytes).map_err(JsonRejection::InvalidJson)?;
        Ok(Json(value))
    }
}
```

**Error discrimination:** `Limited`'s body error is boxed and can be either a `LengthLimitError` (body too large) or an underlying transport error. Only the former becomes 413; other errors become 400 Bad Request. This matches the correct HTTP semantics.

Default limit: 256 KiB (`const DEFAULT_BODY_LIMIT: usize = 262_144`). Runtime-configurable via `ServerConfig::json_body_limit` -> `RequestContext::body_limit`.

### Router

`HashMap<Method, matchit::Router<Arc<dyn Endpoint<S>>>>` for method dispatch + path matching. On match: copy params to owned `RouteParams`, insert into extensions, dispatch to endpoint.

### Middleware: fully owned Arc-based chain

The middleware chain must be fully owned so boxed futures can be `'static`. No borrowed slices in the chain:

```rust
pub trait Middleware<S>: Send + Sync + 'static {
    fn call(&self, req: Request, state: Arc<S>, next: Next<S>) -> BoxFuture;
}

pub struct Next<S> {
    endpoint: Arc<dyn Endpoint<S>>,
    middleware: Arc<[Arc<dyn Middleware<S>>]>,
    index: usize,  // current position in the middleware stack
}

impl<S: Send + Sync + 'static> Next<S> {
    pub fn run(self, req: Request, state: Arc<S>) -> BoxFuture {
        if self.index < self.middleware.len() {
            let mw = self.middleware[self.index].clone();
            let next = Next {
                endpoint: self.endpoint.clone(),
                middleware: self.middleware.clone(),
                index: self.index + 1,
            };
            mw.call(req, state, next)
        } else {
            // Endpoint also takes Arc<S> — uniform ownership model
            self.endpoint.call(req, state)
        }
    }
}
```

Key details:
- `Arc<[Arc<dyn Middleware<S>>]>` — the slice itself is shared, each middleware is individually Arc'd
- `Next` is fully owned — captures can move into `'static` boxed futures
- `App` stores `Arc<[Arc<dyn Middleware<S>>]>` making App cheaply cloneable
- Both `Endpoint::call` and `Middleware::call` take `Arc<S>` — uniform ownership model, no `&S` vs `Arc<S>` mismatch anywhere in the erased layer

### ServerConfig: explicit HTTP/1 knobs with timer wiring

```rust
pub struct ServerConfig {
    pub addr: String,
    pub json_body_limit: usize,                 // default 256 KiB
    pub keep_alive: bool,                       // default true
    pub header_read_timeout: Option<Duration>,   // requires TokioTimer
    pub max_headers: Option<usize>,             // cap header count for embedded
    pub enable_default_tracing: bool,           // default true
}
```

**Timer contract:** hyper's `http1::Builder` panics at runtime if any timeout option is configured without a timer. This is not optional — `TokioTimer` (from hyper-util) must be wired unconditionally when Flowgate supports timeouts. Since we expose `header_read_timeout`, we wire the timer as part of the builder setup contract:

```rust
let mut builder = http1::Builder::new();
builder.keep_alive(config.keep_alive);

// Always wire timer — it's the contract for any timeout support.
// TokioTimer is zero-cost if no timeouts are actually set.
builder.timer(TokioTimer::new());

if let Some(timeout) = config.header_read_timeout {
    builder.header_read_timeout(timeout);
}

if let Some(max) = config.max_headers {
    builder.max_headers(max);
}
```

Wiring the timer unconditionally is safer than guarding it behind `if timeout.is_some()` — if a future config knob adds another timeout, forgetting to add the timer guard would cause a panic.

## Implementation Order

### Phase 1: Skeleton + body types + erased handler boundary
1. Create `Cargo.toml` with all dependencies and feature flags
2. Create `src/body.rs` — `RequestBody = Incoming`, `ResponseBody = Full<Bytes>`, `Request`, `Response` type aliases
3. Create `src/response.rs` — `IntoResponse` trait + impls for String, &str, StatusCode, (StatusCode, String)
4. Create `src/error.rs` — framework error types
5. Create `src/handler.rs` — `Handler<T,S>` trait, `Endpoint<S>` object-safe trait, zero-arg adapter, bridge struct
6. Create `src/lib.rs` — module declarations and re-exports
7. Verify: `cargo check` passes

### Phase 2: State threading with FromRef + router with owned param extensions
8. Create `src/extract/mod.rs` — `FromRequest`, `FromRequestParts`, `FromRef` traits
9. Create `src/extract/state.rs` — `State<T>` extractor via `FromRef<S>` projection
10. Create `src/context.rs` — `RequestContext` and `RouteParams` types
11. Create `src/router.rs` — Router with matchit, method routing, `get()`/`post()` helpers, owned `RouteParams` in extensions via `RequestContext`
12. Create `src/app.rs` — App builder with `Arc<S>` state, `.with_state()`, `.route()`
13. Verify: `cargo check` passes

### Phase 3: JSON extractor with body size limit
14. Create `src/extract/json.rs` — Json<T> extractor reading limit from `RequestContext`, proper `LengthLimitError` discrimination + `IntoResponse` for Json<T>
15. Expand `src/handler.rs` — `macro_rules!` to generate Handler impls for 1..8 extractors
16. Verify: `cargo check` passes

### Phase 4: Middleware (Arc-based chain) + config + server
17. Create `src/middleware.rs` — Middleware trait with fully owned `Next<S>`, `TracingMiddleware`
18. Create `src/config.rs` — ServerConfig with HTTP/1 knobs + body limit
19. Create `src/server.rs` — `Server::bind().serve()` with hyper accept loop, explicit `http1::Builder` config, `TokioTimer` wired unconditionally
20. Update `src/app.rs` — add `.layer()` method, wire Arc-based middleware chain into request pipeline
21. Update `src/lib.rs` — complete re-exports
22. Verify: `cargo build` passes

### Phase 5: Example + docs
23. Create `examples/hello.rs` — full example with AppState, FromRef for sub-state, health GET, echo POST with JSON, tracing
24. Verify: `cargo run --example hello` works, test with curl
25. Create `docs/architecture.md` — document design decisions, layer map, dependency rationale

### Phase 6: Polish
26. Add doc comments on all public types and functions
27. Add basic tests — router matching, JSON serialization/deserialization, extractor behavior, body limit enforcement, RouteParams extraction
28. Run `cargo clippy`, fix warnings
29. Run `cargo doc --no-deps`, verify doc output is clean
30. Final review of public API surface

## Feature Flags

```toml
[features]
default = ["tracing-fmt"]
tracing-fmt = ["dep:tracing-subscriber"]  # Default tracing setup
ws = ["dep:tokio-tungstenite"]            # WebSocket (stubbed in v0.1)
tls = ["dep:tokio-rustls", "dep:rustls"]  # TLS (deferred)
multi-thread = ["tokio/rt-multi-thread"]  # Multi-threaded runtime
```

## Deferred to v0.2

- WebSocket implementation (v0.1: feature flag + type stubs only)
- Static file serving implementation
- Path<T> and Query<T> extractors (route params already captured in owned extensions)
- Tower compatibility adapter
- TLS/rustls integration
- HTTP/2 support
- Graceful shutdown
- Response streaming (enum body type)
- Workspace split into sub-crates
- Proc-macro DX improvements
- Route-scoped middleware

## Verification

1. `cargo build` — compiles with no errors
2. `cargo build --all-features` — compiles with all feature flags
3. `cargo clippy` — no warnings
4. `cargo test` — all tests pass
5. `cargo run --example hello` — server starts, responds to:
   - `curl http://localhost:8080/health` returns JSON health response with sub-state extraction
   - `curl -X POST http://localhost:8080/echo -H 'Content-Type: application/json' -d '{"msg":"hi"}'` returns echoed JSON
   - Oversized body (> 256 KiB) returns 413 Payload Too Large
   - Request/response logged via tracing
6. `cargo doc --no-deps` — clean documentation output
