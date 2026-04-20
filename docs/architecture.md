# Flowgate Architecture

## Overview

Flowgate is a Rust web framework for embedded Linux systems with FastAPI-inspired ergonomics. It prioritizes low resource usage, compile-time safety, and a minimal dependency footprint.

## Layer Architecture

```
┌─────────────────────────────────┐
│         Application             │  app.rs, config.rs, group.rs
│  (App builder, groups, config)  │
├─────────────────────────────────┤
│          Middleware              │  middleware/
│  Pre-routing: RequestId, custom │
│  Post-routing: Tracing, Timeout │
├─────────────────────────────────┤
│      Extraction / Response      │  extract/*, response.rs, sse.rs
│  (Json, Path, Query, State,     │
│   RequestId, Sse, WebSocket)    │
├─────────────────────────────────┤
│           Routing               │  router.rs
│   (matchit radix trie, params)  │
├─────────────────────────────────┤
│     Protocol / Transport        │  server.rs, body.rs, tls.rs, ws.rs
│  (hyper HTTP/1.1, TCP accept,   │
│   rustls, WebSocket upgrade)    │
├─────────────────────────────────┤
│        Infrastructure           │  error.rs, context.rs
│  (Rejections, RequestContext,   │
│   BoxError)                     │
├─────────────────────────────────┤
│         OpenAPI (optional)      │  openapi/ (feature-gated)
│  (OperationMeta, spec, docs UI) │
└─────────────────────────────────┘
```

## Key Design Decisions

### Builder/Runtime Split with Finalization

The most important structural decision is the **builder-time / runtime split**.

- **Builder time** (`App<S>`): Routes, groups, middleware, and metadata are accumulated raw. Builder method order does not affect semantics — `.layer()` added after `.get()` still applies to all routes.
- **Finalization** (called internally by `serve()`): Flattens groups, merges app middleware into each route's middleware chain, builds the matchit router, produces the route manifest, and generates the OpenAPI spec if enabled.
- **Runtime** (`RuntimeInner<S>`): Frozen, optimized state shared across all connections. No more structural changes after finalization.

This eliminates "must call last" API rules and makes the builder order-insensitive.

### Handler Erasure: Two-Layer Design

- **`Handler<T, S>`** — generic user-facing trait for type inference on extractor tuples
- **`Endpoint<S>`** — object-safe trait stored in the router as `Arc<dyn Endpoint<S>>`
- A bridge struct (`HandlerEndpoint`) converts between the two via `.clone()` + `Box::pin()`

### Ownership Model

- `Arc<S>` throughout the erased/middleware layer (Endpoint, Middleware, Next)
- `&S` for extractors — the Handler impl dereferences `&*state` before calling extractors
- `Arc<[Arc<dyn Middleware<S>>]>` for middleware chains — cheaply cloneable

### Route Groups

Groups carry a path prefix, middleware, and tags. They nest arbitrarily and are flattened at finalization:

- Path prefixes are concatenated with a shared `normalize_group_path()` routine
- Middleware stacks merge: parent group → child group → route-local
- Tags are inherited: parent → child → route
- App-level middleware is merged last at finalization (order-insensitive)

### Pre-routing vs Post-routing Middleware

- **`PreMiddleware<S>`** — runs before route matching. No access to route params. Uses `PreNext<S>` chain with a dispatch closure compiled once at startup.
- **`Middleware<S>`** — runs after route matching. Has access to `RequestContext` with route params and body limit. Uses `Next<S>` chain walking to the endpoint.

The dispatch closure is `Arc<dyn Fn(Request, Arc<S>) -> BoxFuture>`, built once during finalization. Pre-middleware does not allocate per request.

### Extractor Design

- `FromRequestParts<S>` — extracts from headers/metadata without consuming the body
- `FromRequest<S>` — extracts from the full request (may consume the body)
- Handler macro: last argument uses `FromRequest`, all preceding use `FromRequestParts`
- `State<T>`, `Path<T>`, `Query<T>`, `RequestId` implement both traits for any-position use

### Sub-State Extraction

`FromRef<S>` trait projects sub-state from the application state:
- `State<AppState>` works via identity impl (`AppState: Clone`)
- `State<Arc<Db>>` works via user-implemented `FromRef<AppState> for Arc<Db>`

### Body Size Limits

- `RequestContext` carries the runtime body limit from `ServerConfig`
- `Json<T>` reads the limit from request extensions
- Uses `http_body_util::Limited` with proper `LengthLimitError` discrimination
- Default: 256 KiB

### HTTP/1 Configuration

- hyper's `http1::Builder` requires explicit `TokioTimer` wiring for any timeout support
- Timer is wired unconditionally to prevent runtime panics
- Explicit knobs for keep-alive, header read timeout, max headers

### OpenAPI (feature-gated)

- Route metadata is **manual** via `OperationMeta` builders
- Schemas are **manual** via `SchemaObject` — no automatic Rust type introspection
- `.with_openapi()` is a declarative toggle; finalization generates the spec and injects `/openapi.json` and `/docs` routes
- When `openapi` feature is off, `OperationMeta` is a zero-size stub with no-op builders — user code compiles identically

### Metrics observer hook

- `MetricsObserver` trait is registered via `App::observe(...)` and invoked once per request from `dispatch_request`, **after** the response is produced
- `RequestEvent` is keyed on the **matched route pattern** (`Arc<str>` stored on `CompiledRoute`), never the raw path — bounded-cardinality labels are the contract for telemetry backends
- 404 and 405 responses emit the event with `route_pattern: None`
- Zero-observer fast path: `dispatch_request` reads `observers.is_empty()` and skips the wall-clock read and pattern clone entirely — confirmed byte-identical in the bench matrix

### TLS wiring (feature-gated)

When the `tls` feature is enabled and `ServerConfig::tls(..)` is set, the accept loop branches on each new connection:

```
listener.accept()
    ↓
spawn task per connection
    ↓
[if tls] TlsAcceptor::accept(stream).await   ← handshake here, NOT in accept task
    ↓
TokioIo::new(stream)
    ↓
hyper::server::conn::http1::Builder::new()
    .with_upgrades()                          ← required for WS
    .serve_connection(io, service)
    .await
```

Key points:

- The `TlsAcceptor` is built **once** outside the accept loop from `Arc<rustls::ServerConfig>` and cloned cheaply into each spawned connection task.
- Handshake runs inside the per-connection task, never the accept task. Slow handshakes do not stall `accept()`.
- On handshake failure, the framework logs at `warn!` and drops the connection; the server keeps running.
- ALPN is forcibly set to `["http/1.1"]` regardless of what the caller-supplied `rustls::ServerConfig` advertised. HTTP/2 is an explicit non-goal for v0.2.
- The plain-TCP and TLS paths share a single generic helper `serve_one_connection<S, IO>` so hyper is driven identically in both cases.

### Streaming response bodies

`ResponseBody` is `UnsyncBoxBody<Bytes, BoxError>` — a type-erased body that supports both buffered and streaming responses. `Unsync` is required because streaming bodies spawned from async user code are almost never `Sync`; hyper requires only `Send`.

Three constructors live in `body.rs`:

- `body::full(bytes)` — buffered body from any `Into<Bytes>` source.
- `body::empty()` — zero-length body.
- `body::stream<S, E>(s)` — streaming body from any `S: Stream<Item = Result<Bytes, E>>` where `E: Into<BoxError>`. Each `Bytes` is wrapped in `Frame::data(..)` and fed into `http_body_util::StreamBody`. Trailers and raw-frame control are deliberately out of scope for v0.2 — a `frames_stream(..)` companion can be added later without breaking this signature.

`Sse<S>` is the first consumer of `body::stream`. The same primitive is available for arbitrary streaming use (chunked downloads, server-pushed event feeds, etc.).

### WebSocket upgrade flow (feature-gated)

When the `ws` feature is enabled, `WebSocketUpgrade` is a dual extractor (`FromRequestParts` **and** `FromRequest`) that:

1. Validates the request headers: `Connection` contains the token `upgrade`, `Upgrade` contains the token `websocket`, `Sec-WebSocket-Version == 13`, `Sec-WebSocket-Key` decodes to 16 bytes. Header tokens are parsed comma-separated and compared case-insensitively — `Connection: keep-alive, Upgrade` is correctly accepted.
2. Computes `Sec-WebSocket-Accept` per RFC 6455 (SHA-1 of `<Key>` ‖ `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`, base64-encoded) and stashes it on the extractor struct.
3. Removes `OnUpgrade` from `parts.extensions` for later use.

Calling `ws.on_upgrade(callback)` builds a `101 Switching Protocols` response (with `Upgrade: websocket`, `Connection: upgrade`, and the pre-computed `Sec-WebSocket-Accept`) and **detached-spawns** a task that:

```
on_upgrade.await         ← hyper completes the upgrade
    ↓
Upgraded stream
    ↓
TokioIo::new(upgraded)
    ↓
WebSocketStream::from_raw_socket(.., Role::Server, None)
    ↓
callback(WebSocket).await
```

The 101 response is returned to hyper before the upgrade task awaits — hyper writes the response, completes the handshake, and resolves `OnUpgrade`. **The hyper connection driver must call `.with_upgrades()` before awaiting**; without it, hyper tears down the socket after writing 101 and the upgrade future errors with `ResetWithoutClosingHandshake`.

`WebSocket` is a thin newtype over `WebSocketStream<TokioIo<Upgraded>>`. `recv() -> Option<Result<Message, WsError>>` delegates to `StreamExt::next`; `send`/`close` delegate to `SinkExt`.

### Graceful shutdown and upgraded connections

`ServerHandle::shutdown()` does two things:

1. **Stop accepting** new TCP connections by dropping the listener.
2. **Drain** active TCP connections within a 30-second budget — wait for in-flight HTTP/1 requests to complete, then drop any connection still hanging on.

Upgraded connections (WebSockets) are an explicit carve-out. The upgrade task is `tokio::spawn`-ed detached — no `JoinHandle` is held, no shutdown signal is plumbed in. Upgraded tasks **survive shutdown** and are not included in the 30-second drain timer.

The reason: making drain wait for WebSocket sessions would pin the shutdown timeout to the longest open WS session, which is not embedded-friendly. Applications that need coordinated session closure should keep a `tokio::sync::broadcast::Sender` in app state and signal it before calling `shutdown()`. A future opt-in `shutdown_signal` knob on `ServerConfig` could automate this, but the carve-out shape is intentional and the rustdoc on `ServerHandle::shutdown` documents it.

## Known Limitations

### RecoverMiddleware is post-routing only

`RecoverMiddleware` implements `Middleware<S>` (post-routing). It catches panics in handlers and post-routing middleware, but **not** in pre-routing middleware or the routing/dispatch logic itself. This is intentional — matchit routing is infallible in practice, and pre-routing middleware panics are better caught by an outer `catch_unwind` at the connection level if needed.

### Response body type

`Response` uses `UnsyncBoxBody<Bytes, BoxError>` — a type-erased body that supports both buffered and streaming responses. Helper functions `body::full()` and `body::empty()` create buffered bodies; `body::stream(s)` turns any `Stream<Item = Result<Bytes, E>>` into a streaming body. See "Streaming response bodies" above for details.

## Dependencies

Required (always built):

| Crate | Purpose |
|-------|---------|
| tokio | Async runtime (single-threaded default) |
| hyper 1.x | HTTP protocol implementation |
| hyper-util | Server utilities, TokioTimer, TokioIo |
| matchit | Zero-allocation radix trie router |
| serde + serde_json | JSON serialization |
| serde_urlencoded | Query string deserialization |
| tracing | Structured logging |
| http, http-body, http-body-util | HTTP types, body wrappers, streaming-body adapters |
| bytes | Zero-copy byte buffers (request/response bodies, RequestId, OpenAPI cache) |
| futures-core | `Stream` trait — used by `body::stream` and `Sse` |
| futures-util | `StreamExt`/`SinkExt` for SSE keep-alive merging, WebSocket I/O, and RecoverMiddleware |

Feature-gated:

| Crate | Feature | Purpose |
|-------|---------|---------|
| tracing-subscriber | `tracing-fmt` (default) | Auto-init structured logging |
| tokio-rustls, rustls, rustls-pemfile | `tls` | TLS 1.2/1.3 server, PEM cert+key parsing |
| tokio-tungstenite, sha1, base64 | `ws` | WebSocket framing + RFC 6455 handshake math |

Dev-only:

| Crate | Purpose |
|-------|---------|
| criterion | Benchmark harness (`benches/dispatch.rs`) |
| rcgen | Self-signed cert generation for TLS tests/example |
| tempfile | Temp PEM files for TLS round-trip tests |
| webpki-roots | Trust anchor for the TLS test client |
| tokio-tungstenite | WebSocket client for integration tests |
