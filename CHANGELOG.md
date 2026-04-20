# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-04-19

The first release big enough to actually deploy. v0.2 closes the production-critical gaps left by v0.1 — TLS, WebSocket, SSE — and lands the ergonomics that turn the framework from a working prototype into something pleasant to build with: route groups, an OpenAPI pipeline, observability hooks, and a per-request hot path that allocates strictly less than v0.1 across every benchmarked shape.

### Highlights

- **TLS via rustls** — feature-gated `tls` flag, `TlsConfig::from_pem_files` for cert+key bundles, `TlsConfig::from_rustls` for ACME / in-memory certs, ALPN pinned to `http/1.1`. Handshake runs inside the per-connection task so slow clients cannot stall the accept loop.
- **WebSocket support** — feature-gated `ws` flag, `WebSocketUpgrade` extractor, RFC 6455 handshake (token-aware `Connection`/`Upgrade` parsing — `Connection: keep-alive, Upgrade` works), detached upgrade tasks. Built on `tokio-tungstenite 0.29`.
- **Server-Sent Events** — always-on `Sse<S: Stream<Item = Event>>` responder with optional keep-alive heartbeats. Backed by a new general-purpose `body::stream(...)` helper.
- **Route groups** — nested groups with prefix, middleware, and tag inheritance; flattened at finalization. Order-insensitive builder: `.layer()` after `.get()` still applies.
- **OpenAPI 3.1 + Scalar docs UI** — feature-gated `openapi` flag. Manual `OperationMeta` builders (no proc macros, no Rust-type introspection); `/openapi.json` + `/docs` injected at finalization.
- **Pre-routing middleware** — `PreMiddleware<S>` runs before route matching for request IDs, auth shortcuts, and path normalization; built-in `RequestIdMiddleware`.
- **Operational middleware** — `TimeoutMiddleware`, `RecoverMiddleware` (feature-gated panic catcher).
- **Metrics observer hook** — `App::observe(...)` registers a `MetricsObserver` that fires once per request with a borrowed `RequestEvent`. Keyed on the *matched route pattern* (not the raw path) for bounded-cardinality labels. Zero cost when no observer is registered.
- **Graceful shutdown** — `ServerHandle::shutdown()` stops accepting new connections and drains active TCP requests on a 30-second budget.
- **Body-read timeout for `Json<T>`** — defends single-threaded embedded deployments against slow-loris bytes-trickling.
- **Per-request allocation reductions across the whole bench matrix** — see the perf section below.

### Fewer per-request allocations across the whole matrix

Every benchmarked request shape now allocates strictly less than v0.1. The heaviest production stacks get the biggest wins:

| Request shape                  | Before | After |
| ------------------------------ | :----: | :---: |
| `GET /health` (no middleware)  | 27     | 26    |
| `GET /users/{id}` (path param) | 32     | 31    |
| `GET /users/{id}` + request-id | 35     | 33    |
| Tracing + request-id + timeout | 39     | 37    |
| `POST /echo` (JSON round-trip) | 41     | 40    |
| 404 (no route matched)         | 23     | 22    |
| `GET /openapi.json`            | 29     | 26    |

The savings come from three targeted changes: the router no longer clones the request path, `RequestId` now shares a single `Bytes`-backed buffer between the request extension and the response header, and OpenAPI spec + Scalar docs bodies are stored as `bytes::Bytes` (clones become refcount increments). See [`docs/perf-baseline.md`](docs/perf-baseline.md) for the full matrix and methodology.

Serving `/openapi.json` on a realistic 52 KB spec is **~15 % faster at p50** (11.80 µs → 10.03 µs).

### TLS

```rust
use flowgate::{App, ServerConfig, TlsConfig};

let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
let config = ServerConfig::new().port(8443).tls(tls);
let app = App::new().get("/", || async { "hello over TLS!" })?;
flowgate::server::serve(app, config).await?;
```

- ALPN advertises **only** `http/1.1`. HTTP/2 is an explicit non-goal for v0.2.
- `from_pem_files` accepts PKCS#8, RSA (PKCS#1), and SEC1 (EC) private keys. Distinct `TlsError` variants for `NoCertificates`, `NoPrivateKey`, `UnsupportedKeyFormat`, `InvalidCertOrKey`, `Io`.
- `from_rustls(Arc<rustls::ServerConfig>)` is the escape hatch for ACME / in-memory certs; ALPN is forcibly overwritten to `http/1.1` regardless of what the caller set.
- Handshake runs **inside the per-connection task**, never the accept task — slow handshakes do not stall `accept()`.

### WebSocket

```rust
use flowgate::{App, WebSocketUpgrade, Message};

async fn echo(ws: WebSocketUpgrade) -> impl flowgate::IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        while let Some(Ok(msg)) = socket.recv().await {
            if matches!(msg, Message::Close(_)) { break; }
            if socket.send(msg).await.is_err() { break; }
        }
    })
}
```

- `WebSocketUpgrade` is both `FromRequestParts` and `FromRequest` — works as the sole handler argument or alongside others.
- Header parsing is **token-aware and case-insensitive**: requests with `Connection: keep-alive, Upgrade` succeed (a regression against naive string equality is covered by an integration test).
- Upgraded tasks run **detached**. They survive `ServerHandle::shutdown()` and are not included in the 30-second drain timer — coordinate clean session closure with a `tokio::sync::broadcast::Sender` in app state if you need it.
- `Sec-WebSocket-Accept` is computed per RFC 6455 (SHA-1 of `<Key>` + `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`, base64-encoded).

### Server-Sent Events

```rust
use std::time::Duration;
use flowgate::sse::{Event, Sse};
use futures_util::stream;

async fn events() -> Sse<impl futures_core::Stream<Item = Event>> {
    let s = stream::unfold(0u64, |n| async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Some((Event::default().id(n.to_string()).data(format!("tick {n}")), n + 1))
    });
    Sse::new(s).keep_alive(Duration::from_secs(15))
}
```

- `Sse<S: Stream<Item = Event>>` always available (no feature gate).
- `Event` builder: `data`, `event`, `id`, `retry(Duration)`. Multiline values produce one `key: value\n` line per line.
- `keep_alive(Duration)` interleaves `:\n\n` comment frames at the configured interval — invisible to clients per the SSE spec, prevents idle-connection drops by intermediaries.
- The response headers set `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `X-Accel-Buffering: no` (the last for nginx-style buffering proxies).
- Backed by `body::stream<S, E>(s) -> ResponseBody where S: Stream<Item = Result<Bytes, E>>` — the same primitive is available for any streaming-body use case.

### Body-read timeout for `Json<T>` extraction

`ServerConfig::body_read_timeout` bounds how long `Json<T>` will wait for request-body bytes. Default: `Some(Duration::from_secs(30))`. When the timeout fires:

- Responds with **`408 Request Timeout`**.
- Sets **`Connection: close`** so the underlying keep-alive socket is dropped.
- `JsonRejection::BodyReadTimeout` is exposed as a public variant for custom error handling.

This defends single-threaded embedded deployments against slow-loris clients that would otherwise hold a worker indefinitely by trickling bytes. Set to `None` to opt out.

### Metrics observer hook — zero cost when unused

`MetricsObserver` trait + `App::observe(...)` registration. Observers receive a borrowed `RequestEvent<'_>` containing:

- the HTTP method,
- the **matched route pattern** (e.g. `/users/{id}`) — never the raw path, for bounded-cardinality metrics labels,
- the response status code,
- wall-clock duration inside the framework.

The event fires uniformly across matched routes, 404s, and 405s (the latter two with `route_pattern: None`). Multiple observers fire in registration order. With no observer registered, the dispatch path does not even read the clock — confirmed by the bench matrix being byte-identical before and after the change.

```rust
use flowgate::{App, MetricsObserver, RequestEvent};

struct Counter;
impl MetricsObserver for Counter {
    fn on_request(&self, event: &RequestEvent<'_>) {
        let label = event.route_pattern.unwrap_or("<unmatched>");
        // atomic increment, channel send, etc. — keep it cheap.
    }
}

let app = App::new()
    .get("/users/{id}", get_user)?
    .observe(Counter);
```

Observer callbacks are **synchronous** and run on the dispatch path — any I/O or expensive work should be forwarded over a channel to a background task.

### Added

- **TLS** (`tls` feature): `TlsConfig`, `TlsError`, `TlsConfig::from_pem_files`, `TlsConfig::from_rustls`, `ServerConfig::tls(..)`. PKCS#8 / PKCS#1 / SEC1 key formats supported.
- **WebSocket** (`ws` feature): `WebSocketUpgrade`, `WebSocket`, `Message` (re-exported from tungstenite), `WsError`. Token-aware `Connection`/`Upgrade` header parsing; RFC 6455 handshake.
- **SSE**: `flowgate::sse::{Event, Sse}`. Builder-style `Event`, `Sse::new(stream).keep_alive(Duration)`. Headers set automatically by the `IntoResponse` impl.
- **Route groups**: `Group<S>` with path prefix, middleware, and tag inheritance. Arbitrarily nestable; flattened at finalization.
- **OpenAPI** (`openapi` feature): `App::with_openapi()`, manual `OperationMeta` builders (`summary`, `tag`, `param`, `body`, `response`), `App::meta(AppMeta)`. Serves `/openapi.json` and `/docs` (Scalar UI). When the feature is off, `OperationMeta` is a zero-size stub with no-op builders.
- **Pre-routing middleware**: `PreMiddleware<S>`, `PreNext<S>`, `App::pre(..)`, built-in `RequestIdMiddleware`.
- **Operational middleware**: `TimeoutMiddleware` (504 on duration exceeded), `RecoverMiddleware` (`recover` feature; converts handler panics to 500), `TracingMiddleware`.
- **Extractors**: `Path<T>` (single, tuple, named struct), `Query<T>`, `RequestId`, `State<T>` with `FromRef` sub-state projection.
- **Graceful shutdown**: `ServerHandle::shutdown()` stops accepting new connections and drains active TCP requests within 30 seconds. Upgraded WebSocket connections are detached and excluded from the drain (documented carve-out).
- **HTTP method coverage**: `App::patch(..)`, `App::options(..)` and the corresponding `_with` variants.
- **Body-read timeout**: `ServerConfig::body_read_timeout: Option<Duration>` (default 30s). `JsonRejection::BodyReadTimeout` → 408 + `Connection: close`.
- **Metrics observer hook**: `MetricsObserver` trait, `RequestEvent<'a>`, `App::observe(..)` (multi-observer).
- **Streaming response bodies**: `body::stream<S, E>(s) -> ResponseBody`. Powers SSE; available for arbitrary streaming use.
- **Type alias**: `BoxError` (`Box<dyn std::error::Error + Send + Sync + 'static>`).
- **Ergonomic re-exports** at crate root: `bytes::Bytes`, `http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode}` — no need for parallel `http` / `bytes` entries in `Cargo.toml`.
- **Examples**: `examples/groups.rs` (groups + OpenAPI), `examples/tls.rs` (HTTPS with self-signed cert), `examples/sse.rs` (counter endpoint), `examples/ws_echo.rs` (WebSocket echo).
- **Benchmarks**: `benches/dispatch.rs` — criterion suite covering router dispatch, middleware stacks, JSON round-trip, OpenAPI spec serving, and 404s.
- **Docs**: `docs/perf-baseline.md` (benchmark methodology + reference matrix); expanded `docs/architecture.md` (TLS, streaming, WebSocket upgrade, shutdown carve-out).

### Changed

- **`ResponseBody` is now `UnsyncBoxBody<Bytes, BoxError>`** (was `BoxBody<Bytes, Infallible>`). Streaming bodies spawned from async user code are almost never `Sync`; hyper requires only `Send`. `body::full()` and `body::empty()` use the same type.
- **OpenAPI module consolidation**: deleted `src/openapi_stub.rs`. `src/openapi/mod.rs` now owns both feature-on and feature-off branches under `#[cfg(..)]`. Same `flowgate::OperationMeta` import path either way.
- **`RequestId` wraps a `HeaderValue` internally** (was `String`). Access via `RequestId::as_str()` or its `Display` impl. `.0` is no longer `pub`.
- **`CompiledRoute` carries its registered pattern as `Arc<str>`** — required by the observer hook for bounded-cardinality keys.
- **OpenAPI spec bytes and Scalar docs HTML are stored as `bytes::Bytes`** (were `Vec<u8>` / `String`) — per-request clones are refcount increments.
- **hyper connection driver calls `.with_upgrades()`** before awaiting — required for the WebSocket upgrade handoff to work.
- **`futures-core` and `futures-util` are now unconditional dependencies**. `futures-util` was previously gated under the `recover` feature. Both are small and used widely in the Tokio ecosystem.

### Internal

- Builder/runtime split via `App::finalize()` — flattens groups, merges app middleware into each route, builds the matchit router, generates the OpenAPI spec, and produces a frozen `RuntimeInner`. Builder method order does not affect semantics.
- `serve_with_listener(app, config, listener)` exposed for tests that need a pre-bound `TcpListener` (avoids the bind / drop / re-bind race).
- All test files use `#[tokio::test(flavor = "current_thread")]`.

### Deferred

Items deliberately not in 0.2.0:

- **Static file serving** — planned as a small, aggressively-scoped follow-up in **0.2.1 or 0.2.2** once the core is dogfed in anger. It's a surface-area feature (path traversal, MIME types, HEAD, index files, SPA fallback, cache headers, conditional GET, ranges, symlink safety) that would have slowed publication.
- **Tower compatibility adapter** — deferred to **v0.3**.
- **HTTP/2** — deferred to **v0.3**. TLS explicitly advertises only `http/1.1` via ALPN.
- **Workspace split into sub-crates** — deferred to **v0.3** if needed at all.
- **Proc-macro DX** — deferred indefinitely. `macro_rules!` remains the handler-generation mechanism.
- **Waiting for upgraded (WebSocket) connections during graceful shutdown** — explicit carve-out, documented on `ServerHandle::shutdown`. Coordinate session closure in application state.

### Tests

Test count grew from **66** (end of v0.1) to **78** at the close of v0.2, plus 2 doc-tests. New coverage includes:

- TLS round-trip (PEM-files path and `from_rustls` path), ALPN enforcement, key-format discrimination.
- WebSocket echo round-trip, compound `Connection: keep-alive, Upgrade` header acceptance, RFC 6455 `Sec-WebSocket-Accept` digest.
- SSE event serialization (single-line, multiline, empty, full-field), heartbeat frame interval, finite stream termination.
- `Json<T>` stalled-body scenario → 408 + `Connection: close`; happy path with the timeout configured.
- Observer captures the matched pattern (not the raw path); 404 and 405 patterns are `None`; multiple observers all fire.
- Extended `ServerConfig` default/builder coverage for new knobs.

## [0.1.0] - 2025-12-13

Initial release. Single-crate web framework on hyper 1.x with type-safe handlers, JSON extraction, and a tracing-enabled HTTP/1 server.

[0.2.0]: https://github.com/alvytsk/flowgate/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/alvytsk/flowgate/releases/tag/v0.1.0
