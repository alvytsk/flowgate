# Flowgate v0.2.0 — Scope, Design, and Release Plan

## Context

The `feature/v0.2` branch is well past the v0.1 scope: router groups, OpenAPI + Scalar UI, benchmarks, `MetricsObserver`, pre-routing middleware, `RequestId`/`Timeout`/`Recover` middleware, Path/Query extractors, graceful shutdown, PATCH/OPTIONS, and body-read timeouts have all landed. No formal v0.2 plan was written — the work grew organically from the v0.1 "deferred" list. The branch is feature-rich but missing production-critical pieces (TLS, WS, SSE) and has not been cut as a release.

The goal of **this** cycle is to close the remaining production gaps — **TLS, WebSocket, and SSE** — polish the architecture, write a retroactive plan doc and changelog, and cut **0.2.0** to crates.io. Version is still `0.1.0` in `Cargo.toml:3` while `README.md` already advertises `flowgate = "0.2"`. This version-mismatch is actively harmful — users forgive missing features far more easily than version confusion — so bumping and publishing takes priority over scope expansion.

## Non-Goals

**Out of 0.2.0 scope. Ship without them.**

| Item | Disposition |
|---|---|
| Static file serving | Deliberately excluded. It's a surface-area feature (path traversal, MIME types, HEAD, index files, SPA fallback, cache headers, conditional GET, ranges, symlink safety) that would slow publication. Planned as a small, aggressively-scoped follow-up in **0.2.1 or 0.2.2** once the core is dogfed in anger. |
| Tower compatibility adapter | Deferred to v0.3 |
| HTTP/2 | Deferred to v0.3 (TLS explicitly advertises only `http/1.1` via ALPN) |
| Workspace split into sub-crates | Deferred to v0.3 |
| Proc-macro DX | Deferred indefinitely — `macro_rules!` remains the handler-generation mechanism |

## Key Design Decisions

| Decision | Choice | Why |
|---|---|---|
| `body::stream()` signature | `stream<S, E>(s: S) -> ResponseBody where S: Stream<Item = Result<Bytes, E>> + Send + 'static, E: Into<BoxError>` | Keeps the public surface on `Bytes`, not `Frame<Bytes>`. Framework internally wraps each `Bytes` in `Frame::data(..)`. Trailers / raw-frame control stay out of v0.2 — can be added via a second helper (`frames_stream`) later without breaking callers |
| `BoxError` type alias | `pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>`, defined in `src/error.rs` | Canonical error-erasure shape for streaming bodies and future extension points |
| Stream dependency | `futures-core` **and** `futures-util` both unconditional | `futures-core` gives us `Stream`; `futures-util` gives us `StreamExt::map`/`merge` needed by SSE keep-alive. Both are small and used widely elsewhere in the Tokio ecosystem. Drops the `recover` feature gate on `futures-util` |
| TLS config shape | Take `Arc<rustls::ServerConfig>` directly, with a `from_pem_files(cert, key)` convenience helper | Keeps cert parsing out of the accept path; escape hatch for ACME / in-memory certs |
| TLS key formats | `from_pem_files()` accepts PKCS#8, RSA (PKCS#1), and SEC1 (EC) keys. Parse with `rustls-pemfile`, walk the item stream, stop at the first private-key item of any supported kind. Distinct `TlsError` variants: `NoPrivateKey`, `UnsupportedKeyFormat`, `CertParseError`, `KeyParseError`, `Io` | Users have real-world keys in all three formats; silently failing on "wrong" format turns the helper into a support burden |
| TLS ALPN | Advertise only `http/1.1` | Matches the HTTP/1-only stance of v0.2; HTTP/2 is explicitly non-goal |
| TLS wiring | Feature-gated branch inside the spawned per-connection task (not the accept task): `TlsAcceptor::accept(stream).await` before `TokioIo::new(..)` | Slow handshakes don't stall `accept()`; TLS failure logs + drops one connection, does not kill the server |
| WebSocket upgrade | `WebSocketUpgrade` extractor (`FromRequestParts`) + `ws.on_upgrade(closure)` returns a 101 response; closure runs after hyper's `OnUpgrade` resolves | Axum-proven ergonomics; keeps the framework out of the handshake data-plane |
| WebSocket header validation | **Token-aware, case-insensitive.** `Connection` parsed as comma-separated tokens, check that `upgrade` is present. `Upgrade` parsed as comma-separated tokens, check that `websocket` is present. `Sec-WebSocket-Version` must equal `13`. `Sec-WebSocket-Key` must be present and decode to 16 bytes | String equality silently rejects `Connection: keep-alive, Upgrade` which is valid and common; "mostly works" is not the bar |
| WebSocket stream | `WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None)` on `hyper::upgrade::Upgraded` | tokio-tungstenite 0.29 supports this; skips redundant handshake (hyper already did it) |
| Upgraded-connection shutdown semantics | **Graceful shutdown stops accepting new TCP connections but does NOT wait for active WebSocket tasks.** Upgraded tasks are spawned detached; the 30-second drain timer ignores them. Clean WS shutdown is the application's responsibility (e.g., a `broadcast::Sender` in app state). This is explicitly documented in the `ServerHandle::shutdown` rustdoc and in `docs/architecture.md` | Pick **one** behavior and write it down. The alternative (drain waits for WS) pins the shutdown timeout to the longest open WS session, which isn't embedded-friendly. We can tighten later with an opt-in `shutdown_signal` broadcast channel on `ServerConfig` — structured so it fits without redesign |
| SSE shape | `Sse<S: Stream<Item = Event>>` with builder-style modifiers: `Sse::new(stream).keep_alive(Duration)`. `IntoResponse` impl sets `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `X-Accel-Buffering: no` | Real-world proxies buffer or drop idle streams; keep-alive is required for robust SSE. Builder shape leaves room for `.retry(Duration)` and `.headers(..)` without redesign |
| SSE keep-alive implementation | Internally build a merged stream: user events interleaved with a `tokio::time::interval` emitting `:\n\n` comment frames. Comment frames are ignored by clients per the SSE spec | No client-visible effect; keeps the socket warm for intermediaries |
| Module layout for new code | Flat: `src/tls.rs`, `src/sse.rs`, `src/ws.rs` | Consistent with current flat style (`body.rs`, `context.rs`); avoid premature nesting |
| `openapi_stub.rs` | Delete; fold into `src/openapi/mod.rs` with `#[cfg(..)]` branches | Removes dual-module weirdness; one entry point in `lib.rs` |

## Files to Modify / Create

**Cargo:**
- `Cargo.toml` — bump to `0.2.0`; add `futures-core = "0.3"` and `futures-util = { version = "0.3", default-features = false, features = ["std", "async-await"] }` unconditional (drop the `recover` gate on `futures-util`); add `rcgen = "0.13"` dev-dep for TLS test certs; add `sha1 = "0.10"` + `base64 = "0.22"` as `ws`-gated deps

**Core edits:**
- `src/lib.rs` — new re-exports (`TlsConfig`, `Sse`, `Event`, `WebSocketUpgrade`, `WebSocket`, `Message`, `BoxError`, `body::{stream, full, empty}`); collapse `openapi` / `openapi_stub` into one `pub mod openapi`
- `src/error.rs` — add `pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;`
- `src/config.rs` — add `#[cfg(feature = "tls")] pub tls: Option<TlsConfig>` field + builder method
- `src/server.rs` — feature-gated TLS branch in the spawned per-connection task; plain TCP path unchanged when `config.tls.is_none()`. Add rustdoc on `ServerHandle::shutdown` documenting the WebSocket carve-out
- `src/body.rs` — add `pub fn stream<S, E>(s: S) -> ResponseBody where S: Stream<Item = Result<Bytes, E>> + Send + 'static, E: Into<BoxError>`. Internally `stream.map(|res| res.map(Frame::data).map_err(Into::into))` into `StreamBody::new(..).boxed()`

**New modules:**
- `src/tls.rs` (`#[cfg(feature = "tls")]`) — `TlsConfig`, `TlsError` (variants as above), `TlsConfig::from_pem_files(cert_path, key_path)` parses with `rustls_pemfile::read_all` and accepts PKCS#8 / RSA / SEC1, `TlsConfig::from_rustls(Arc<rustls::ServerConfig>)`
- `src/sse.rs` (unconditional) — `Event` builder (`.data()`, `.event()`, `.id()`, `.retry()`); `Sse<S>` wrapper holding `{ stream: S, keep_alive: Option<Duration> }`; `Sse::new`, `Sse::keep_alive(Duration)`; `IntoResponse for Sse<S>` — serializes events as `data:`/`event:`/`id:`/`retry:` framing, optional keep-alive interleaves `:\n\n` heartbeats via `StreamExt` merge
- `src/ws.rs` (`#[cfg(feature = "ws")]`) — `WebSocketUpgrade` (extractor), `WebSocket`, `Message` (re-export from tungstenite), `WsError`; token-aware header validation with shared helpers (`header_contains_token(headers, name, token)`) for `Connection` and `Upgrade`; `Sec-WebSocket-Accept` via SHA-1 + base64 of `<Key><GUID>` where GUID is `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`

**Delete / consolidate:**
- `src/openapi_stub.rs` — remove; fold stub into `src/openapi/mod.rs` under `#[cfg(not(feature = "openapi"))]`

**Examples:**
- `examples/tls.rs` (req: `--features tls`) — HTTPS hello, self-signed cert generated at startup via `rcgen`
- `examples/sse.rs` — `/events` endpoint emitting a counter every second with `keep_alive(15s)` enabled
- `examples/ws_echo.rs` (req: `--features ws`) — echo server

**Tests (all in `tests/integration.rs` unless noted):**
- `tls_round_trip_https` — bind random port, self-signed cert, `tokio-rustls` client GET + assert 200
- `tls_accepts_pkcs8_and_rsa_keys` — fixture keys in both formats, each loads successfully
- `sse_stream_emits_events` — collect frames from `/events`, assert `data:` lines, correct `Content-Type`, and that a heartbeat comment frame appears within the configured interval
- `ws_echo_round_trip` (`#[cfg(feature = "ws")]`) — tokio-tungstenite client, echo a text frame
- `ws_accepts_compound_connection_header` — request with `Connection: keep-alive, Upgrade` succeeds (regression against naive string equality)

**Docs:**
- `docs/architecture.md` — add sections on TLS wiring, SSE body streaming, WebSocket upgrade flow, upgraded-connection shutdown semantics; update dependency table
- `CHANGELOG.md` — new file, `0.2.0` entry listing all features landed since `0.1.0`, plus "Deferred to v0.2.x" (static files) and "Deferred to v0.3" (Tower, HTTP/2, workspace split) footnotes
- `README.md` — add TLS / SSE / WebSocket code snippets; bump install snippet to `0.2.0`; update feature table

## Arch Polish Checklist

1. **Delete `openapi_stub.rs`** — single `pub mod openapi;` in `lib.rs` with cfg-gated contents inside
2. **Public API audit in `lib.rs`** — confirm everything a user actually touches is re-exported; demote `RawRoute`, `CompiledRoute`, `RuntimeInner` to `pub(crate)` if they aren't already
3. **Ergonomic re-exports** — add `pub use http::{Method, StatusCode, header};` and `pub use bytes::Bytes;` so users don't need parallel `http` / `bytes` imports
4. **Doc comments** — every `pub` item in `app.rs`, `group.rs`, `middleware/mod.rs`, `observer.rs`, `extract/*.rs`, `server.rs` gets a rustdoc line (most already have them — this is a sweep, not a rewrite)
5. **Clippy zero-warnings** — `cargo clippy --all-targets --all-features -- -D warnings`
6. **`cargo doc --no-deps --all-features`** renders clean with no broken intra-doc links
7. **Leave module layout flat** — resist the urge to create `runtime/`, `core/`, or similar during this release; reorg is a v0.3 concern if needed

## Implementation Order

Each step below is split into a **summary** (one-line goal) and an **extended** description (what the step actually produces, edge cases it covers, and the verification point that proves it's done). Keep the summary terse; use the extended text when executing so intent doesn't drift.

### Phase 1 — TLS (`tls` feature) ✅ COMPLETE

**Step 1 — Add `BoxError` type alias.** ✅
_Extended:_ In `src/error.rs`, introduce `pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>`. Re-export from `lib.rs`. This is the shared error-erasure shape used by `body::stream()` (Phase 2) and any future streaming-body surface. No runtime behavior changes; this step is purely a scaffolding primitive that lets later steps compile without a type-naming detour. Verified by `cargo build` still passing.

**Step 2 — Create `src/tls.rs` with `TlsConfig` + `TlsError`.** ✅
_Extended:_ Feature-gated module (`#[cfg(feature = "tls")]`). `TlsConfig` wraps `Arc<rustls::ServerConfig>` with ALPN pinned to `["http/1.1"]`. `TlsError` enum: `NoCertificates`, `NoPrivateKey`, `UnsupportedKeyFormat`, `InvalidCertOrKey(rustls::Error)`, `Io(std::io::Error)`. Two constructors — `from_pem_files(cert_path, key_path)` and `from_rustls(Arc<rustls::ServerConfig>)`. The PEM loader walks `rustls_pemfile::read_all` and accepts PKCS#8 / RSA (PKCS#1) / SEC1 private-key items, returning `NoPrivateKey` when nothing key-shaped was found and `UnsupportedKeyFormat` when an unrecognized key-like item was present. `from_rustls` overwrites caller-supplied ALPN to guarantee HTTP/1.1 only.

**Step 3 — Thread `tls: Option<TlsConfig>` through `ServerConfig`.** ✅
_Extended:_ Field added under `#[cfg(feature = "tls")]`; default `None`; builder method `.tls(TlsConfig)`. No other config knobs changed. Default-feature build unaffected.

**Step 4 — Wire TLS acceptance into the per-connection task.** ✅
_Extended:_ In `src/server.rs`, extracted a generic `serve_one_connection<S, IO>` helper so both the plain-TCP and TLS paths drive hyper identically. `TlsAcceptor::from(cfg.inner())` is built once outside the accept loop and cloned into each spawned task. On handshake error: `tracing::warn!("tls handshake failed from {peer_addr}: {err}")` and drop — the server keeps running. The handshake runs inside the connection task, so slow clients cannot stall `accept()`.

**Step 5 — Confirm ALPN is advertised.** ✅
_Extended:_ Four unit tests in `src/tls.rs::tests`: ALPN is `["http/1.1"]` after `from_pem_files`, ALPN is overwritten to `["http/1.1"]` when `from_rustls` receives a config with `["h2", "http/1.1"]` preset, `NoCertificates` is returned for an empty cert file, `NoPrivateKey` is returned for an empty key file.

**Step 6 — Build `examples/tls.rs`.** ✅
_Extended:_ Generates a self-signed cert via `rcgen::generate_simple_self_signed(vec!["localhost"])` at startup, converts the key to PKCS#8 DER, builds a `rustls::ServerConfig`, wraps it in `TlsConfig::from_rustls`, runs a hello handler on `127.0.0.1:8443`. Gated via `[[example]] required-features = ["tls"]` in `Cargo.toml`.

**Step 7 — Add TLS integration tests.** ✅
_Extended:_ Two `#[cfg(feature = "tls")]` tests in `tests/integration.rs::tls_tests`: `tls_round_trip_https` binds a random port, serves with an rcgen self-signed cert, drives a `tokio-rustls` client that trusts the generated cert via a custom `RootCertStore`, and asserts 200 + body. `tls_from_pem_files_round_trip` writes the cert+key to tempfiles and exercises the `from_pem_files` path end-to-end. Both pass under `cargo test --features tls`.

### Phase 1 — How to test

```bash
# Unit tests (ALPN + key-format discrimination)
cargo test --features tls --lib tls::

# Integration round-trips (HTTPS client against self-signed server)
cargo test --features tls --test integration tls_tests

# Full suite across all features
cargo test --all-features

# Manual smoke test against the example
cargo run --example tls --features tls
# (in another terminal)
curl -k https://localhost:8443/
# expected: "hello over TLS!"
```

All 4 unit tests and 2 integration tests pass; the full `--all-features` run is 74 tests, zero failures.

### Phase 1 — Summary of changes

**New files**

- `src/tls.rs` — `TlsConfig`, `TlsError`, PEM loader for PKCS#8 / RSA / SEC1, ALPN enforcement, 4 unit tests
- `examples/tls.rs` — self-signed HTTPS example on `:8443`

**Modified files**

- `Cargo.toml` — `rustls-pemfile` under the `tls` feature; dev-deps: `rcgen`, `tokio-rustls`, `rustls`, `rustls-pemfile`, `tempfile`, `webpki-roots`, plus `tokio` with `io-util` (fixes a pre-existing build break in the raw-socket timeout test); `[[example]] tls` entry gated on the `tls` feature
- `src/error.rs` — `pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>`
- `src/lib.rs` — `pub mod tls` (feature-gated); re-exports `BoxError`, `TlsConfig`, `TlsError`
- `src/config.rs` — `tls: Option<TlsConfig>` field and `.tls(TlsConfig)` builder method (feature-gated)
- `src/server.rs` — extracted `serve_one_connection<S, IO>` helper generic over IO, wired a TLS branch that `TlsAcceptor::accept`s inside the per-connection spawn before handing to hyper
- `tests/integration.rs` — `tls_tests` module with `tls_round_trip_https` and `tls_from_pem_files_round_trip`

### Phase 2 — SSE (always on) ✅ COMPLETE

**Step 8 — Add `body::stream()` helper.** ✅
_Extended:_ In `src/body.rs`, added `pub fn stream<S, E>(s: S) -> ResponseBody where S: Stream<Item = Result<Bytes, E>> + Send + 'static, E: Into<BoxError>`. Each item is mapped into `Result<Frame<Bytes>, BoxError>` via `Frame::data(..)` + `Into::into`, then wrapped in `StreamBody::new(..).boxed_unsync()`. Companion change: `ResponseBody` is now `UnsyncBoxBody<Bytes, BoxError>` (was `BoxBody<Bytes, Infallible>`). Unsync is needed because streaming bodies spawned from async user code are almost never `Sync`; hyper requires only `Send`. `full()` and `empty()` use the same type via `Full::new(..).map_err(|i: Infallible| match i {}).boxed_unsync()`. Trailers and raw-frame control are out of scope for v0.2 — a `frames_stream()` companion can be added later without breaking this one.

**Step 9 — Build the SSE module (`src/sse.rs`).** ✅
_Extended:_ Unconditional module, no feature gate. `Event` is a plain struct with chainable builders `data`, `event`, `id`, `retry(Duration)`; `to_bytes()` produces the wire representation (multiline values produce multiple `key: value\n` lines, terminated by a blank line). `Sse<S>` holds `{ stream, keep_alive: Option<Duration> }`, with `Sse::new(stream)` and `.keep_alive(Duration)` builder methods. The `IntoResponse` impl sets `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `X-Accel-Buffering: no`. When `keep_alive` is `Some`, a custom `SseStream` combinator polls user events first and interleaves heartbeat comment frames (`:\n\n`) on every tick — **crucially, the response ends as soon as the user stream ends**; heartbeat alone cannot keep the connection open. Final body goes through `body::stream()`. Four unit tests cover single-line, multiline, full-field, and empty-event serialization.

**Step 10 — Build `examples/sse.rs`.** ✅
_Extended:_ Handler returns `Sse::new(ticker.boxed()).keep_alive(Duration::from_secs(15))` where `ticker` is a `stream::unfold` that sleeps 1 second, yields `Event::default().id(n).data(format!("tick {n}"))`, and repeats. Doc-comment notes `curl -N http://localhost:8080/events` as the manual smoke test.

**Step 11 — Add SSE integration tests.** ✅
_Extended:_ Two tests in `tests/integration.rs::sse_tests`. `sse_stream_emits_events` registers a finite 3-element `stream::iter` stream, makes a request, and asserts status 200, all three content-type-style headers (`text/event-stream`, `no-cache`, `x-accel-buffering: no`), and the full concatenated body `"data: one\n\ndata: two\n\ndata: three\n\n"`. `sse_heartbeat_frames_are_emitted` uses `stream::pending::<Event>()` with a 30ms keep-alive interval, reads exactly one body frame with a 2-second timeout, and asserts it equals `":\n\n"`. Both pass on every test run.

### Phase 2 — How to test

```bash
# Unit tests (Event wire-format serialization)
cargo test --lib sse::

# Integration round-trips (full HTTP + body read)
cargo test --test integration sse_tests

# Full suite (all features including SSE + TLS)
cargo test --all-features

# Manual smoke test against the example
cargo run --example sse
# (in another terminal)
curl -N http://localhost:8080/events
# expected: one "id: N / data: tick N" pair per second, forever
```

All 4 SSE unit tests and 2 SSE integration tests pass; the full `--all-features` run is now 76 tests, zero failures. Clippy is clean at `-D warnings`.

### Phase 2 — Summary of changes

**New files**

- `src/sse.rs` — `Event` + builder, `Sse<S>` wrapper with `keep_alive(Duration)`, `IntoResponse for Sse<S>`, custom `SseStream` combinator that terminates on event-stream end, heartbeat stream built on `tokio::time::Interval`, 4 unit tests
- `examples/sse.rs` — `/events` endpoint emitting one tick/sec with `id:` and `data:` fields, 15s keep-alive

**Modified files**

- `Cargo.toml` — `futures-core` and `futures-util` are now unconditional (dropped the `recover` gate on `futures-util`; the `recover` feature itself still exists but is empty since all gated code already used `futures_util` which is now always available); added `[[example]] sse`
- `src/body.rs` — `ResponseBody` is now `UnsyncBoxBody<Bytes, BoxError>`; added `pub fn stream<S, E>(...)` streaming-body helper; `full()` and `empty()` updated to match the new type
- `src/lib.rs` — `pub mod sse` (unconditional); re-exports `Event` and `Sse`
- `tests/integration.rs` — new `sse_tests` module with `sse_stream_emits_events` and `sse_heartbeat_frames_are_emitted`

### Phase 3 — WebSocket (`ws` feature) ✅ COMPLETE

**Step 12 — Scaffold `src/ws.rs` with the `WebSocketUpgrade` extractor.** ✅
_Extended:_ Feature-gated module. `sha1 = "0.10"` and `base64 = "0.22"` added as `ws`-gated deps. `WebSocketUpgrade` holds `{ on_upgrade: OnUpgrade, sec_accept: HeaderValue }`. `FromRequestParts<S>` impl validates headers via a shared helper `header_contains_token(headers, name, token)` that parses comma-separated, case-insensitive tokens — used for both `Connection` contains `upgrade` and `Upgrade` contains `websocket`. Further checks: `Sec-WebSocket-Version == 13`, `Sec-WebSocket-Key` present and base64-decodes to exactly 16 bytes. Extracts `OnUpgrade` via `parts.extensions.remove::<OnUpgrade>()`. On failure returns a `WsError` that `IntoResponse`s to 400 with a clear body. Also impls `FromRequest<S>` so `WebSocketUpgrade` can sit alone as a handler argument (the last-arg-is-FromRequest rule).

**Step 13 — Compute `Sec-WebSocket-Accept`.** ✅
_Extended:_ `sec_websocket_accept(key)` concatenates the raw `Sec-WebSocket-Key` bytes + GUID `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`, SHA-1s the result, base64-encodes the 20-byte digest, and returns a `HeaderValue`. Stored in the extractor struct so `on_upgrade` can set it on the 101 response without re-reading request headers. Unit test `sec_websocket_accept_rfc_6455_example` validates: `dGhlIHNhbXBsZSBub25jZQ==` → `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`.

**Step 14 — Implement `on_upgrade(callback)` returning a 101 Response.** ✅
_Extended:_ `on_upgrade<F, Fut>(self, callback: F) -> Response` where `F: FnOnce(WebSocket) -> Fut + Send + 'static`. Builds a `101 Switching Protocols` response with `Upgrade: websocket`, `Connection: upgrade`, and the pre-computed `Sec-WebSocket-Accept` header, empty body. Before returning the response, `tokio::spawn`s a **detached** task: awaits `on_upgrade.await`, wraps `Upgraded` via `TokioIo::new`, builds `WebSocketStream::from_raw_socket(.., Role::Server, None)`, invokes `callback(WebSocket { inner }).await`. On upgrade error, `tracing::warn!` and drop — never panics. The task is not tracked by any connection counter, and survives `ServerHandle::shutdown` (documented below). **Critical prerequisite:** `src/server.rs` now calls `.with_upgrades()` on the hyper connection — without it hyper tears down the socket after writing the 101 and the upgrade future errors with `ResetWithoutClosingHandshake`.

**Step 15 — Add the `WebSocket` wrapper and `Message` re-export.** ✅
_Extended:_ `WebSocket` is a thin newtype over `WebSocketStream<TokioIo<Upgraded>>`. Exposes async methods `recv() -> Option<Result<Message, WsError>>`, `send(Message) -> Result<(), WsError>`, `close() -> Result<(), WsError>` — the `send`/`close` paths delegate to `SinkExt`, `recv` delegates to `StreamExt::next`. `Message` re-exported from `tokio_tungstenite::tungstenite`. All surfaced through `flowgate::ws::{Message, WebSocket, WebSocketUpgrade, WsError}` plus top-level re-exports in `lib.rs`.

**Step 16 — Document WS shutdown carve-out on `ServerHandle::shutdown`.** ✅
_Extended:_ Rustdoc on `ServerHandle::shutdown` now states explicitly that upgraded connections run as detached tasks, survive shutdown, and are not included in the 30-second drain timer. Recommends a `tokio::sync::broadcast::Sender` in app state for applications that need to coordinate session closure.

**Step 17 — Build `examples/ws_echo.rs`.** ✅
_Extended:_ Simple echo server on `/ws`; `ws_handler` returns `upgrade.on_upgrade(|mut socket| async move { ... })` with a loop that breaks on close or send error. Gated via `[[example]] required-features = ["ws"]` in `Cargo.toml`.

**Step 18 — Add WS integration tests.** ✅
_Extended:_ Two `#[cfg(feature = "ws")]` tests in `tests/integration.rs::ws_tests`. `ws_echo_round_trip` registers the echo handler, connects via `tokio_tungstenite::client_async` over a raw `TcpStream`, sends `Message::Text("hello")`, asserts the echoed message matches. `ws_accepts_compound_connection_header` is the naive-string-equality regression: crafts a raw HTTP/1.1 upgrade request with `Connection: keep-alive, Upgrade` and the RFC 6455 canonical `Sec-WebSocket-Key`, reads the response bytes, and asserts both the `101 Switching Protocols` status and the canonical `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=` accept value. Both pass.

### Phase 3 — How to test

```bash
# Unit tests (handshake math + header parsing)
cargo test --features ws --lib ws::

# Integration round-trips (echo + compound Connection header)
cargo test --features ws --test integration ws_tests

# Full suite
cargo test --all-features

# Manual smoke test against the example
cargo run --example ws_echo --features ws
# (in another terminal, with websocat installed)
websocat ws://localhost:8080/ws
# type any line; expect it echoed back
```

All 5 WS unit tests and 2 WS integration tests pass. Full `--all-features` run is now **78 tests, zero failures**. Clippy is clean at `-D warnings`.

### Phase 3 — Summary of changes

**New files**

- `src/ws.rs` — `WebSocketUpgrade` extractor (FromRequestParts **and** FromRequest), `WebSocket` wrapper with `recv`/`send`/`close`, `WsError` with `IntoResponse` (400), `Message` re-export, token-aware header helper, RFC 6455 `Sec-WebSocket-Accept` computation, 5 unit tests (RFC example + header-token variants)
- `examples/ws_echo.rs` — echo server on `/ws`, `websocat`-testable

**Modified files**

- `Cargo.toml` — `ws` feature now enables `sha1`, `base64`, and `tokio-tungstenite`; added `[[example]] ws_echo required-features = ["ws"]`; added `tokio-tungstenite` as an unconditional dev-dep for integration-test client; `futures-util` got the `sink` feature (already used transitively)
- `src/server.rs` — hyper connection driver now calls `.with_upgrades()` before awaiting — **required** for upgrade handoff to work; `ServerHandle::shutdown` rustdoc documents the upgraded-task carve-out
- `src/lib.rs` — `pub mod ws` (ws-gated); re-exports `Message`, `WebSocket`, `WebSocketUpgrade`, `WsError`
- `tests/integration.rs` — `ws_tests` module with `ws_echo_round_trip` and `ws_accepts_compound_connection_header`

### Phase 4 — Arch polish

**Step 19 — Consolidate `openapi_stub.rs` into `openapi/mod.rs`.**
_Extended:_ Delete `src/openapi_stub.rs`. Inside `src/openapi/mod.rs`, move the current feature-on contents under `#[cfg(feature = "openapi")] mod spec; #[cfg(feature = "openapi")] pub use spec::*;` (similar for `meta` and `ui` submodules), and add a `#[cfg(not(feature = "openapi"))]` block containing the zero-sized `OperationMeta` stub. Update `src/lib.rs` to have a single unconditional `pub mod openapi;` and remove the `openapi_stub` branch. Verified by `cargo build` (no feature) and `cargo build --features openapi` both passing; the user-facing `OperationMeta` import path is unchanged.

**Step 20 — Add ergonomic re-exports to `lib.rs`.**
_Extended:_ Append `pub use bytes::Bytes;`, `pub use http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};`, and `pub use error::BoxError;`. This eliminates parallel `use http::...` / `use bytes::...` lines in downstream code. Verified by updating `examples/hello.rs` and `examples/groups.rs` to import only from `flowgate::` and watching them still compile.

**Step 21 — Public API surface audit.**
_Extended:_ Walk `src/app.rs`, `src/group.rs`, `src/router.rs`, `src/server.rs` and confirm that internal types (`RawRoute`, `CompiledRoute`, `RuntimeInner`, `FinalizedApp`, etc.) are `pub(crate)`, not `pub`. Anything public that no external user would reasonably call gets demoted. The goal is a minimal 0.2.0 API surface, since anything public we ship becomes a compatibility promise. Verified by `cargo doc --no-deps --all-features` — inspect the generated rustdoc index and confirm it matches what a user should see.

**Step 22 — Doc-comment sweep.**
_Extended:_ For every `pub` item in `app.rs`, `group.rs`, `middleware/mod.rs`, `observer.rs`, `extract/*.rs`, `server.rs`, ensure there is at least a one-line `///` comment. Most items already have one — skip anything already documented. No code changes; rustdoc only. Verified by `cargo doc --no-deps --all-features` rendering clean with no missing-docs warnings.

**Step 23 — Clippy clean.**
_Extended:_ Run `cargo clippy --all-targets --all-features -- -D warnings`. Fix any warnings introduced by the new modules (likely `needless_return`, `redundant_clone`, `uninlined_format_args` patterns). No `#[allow(..)]` without a comment explaining why. Verified by the command exiting 0.

**Step 24 — Doc build clean.**
_Extended:_ Run `cargo doc --no-deps --all-features` and fix any broken intra-doc links (`[Foo]` that don't resolve) or warnings. The generated HTML is the canonical API reference for the release. Verified by the command exiting 0 with no warnings.

### Phase 5 — Release prep

**Step 25 — Bump `Cargo.toml` version to `0.2.0`.**
_Extended:_ Single-line change. Resolves the `README.md` / `Cargo.toml` version mismatch, which is the main blocker for publication. Verified by `cargo build` and `cargo package --allow-dirty` both passing.

**Step 26 — Write `CHANGELOG.md`.**
_Extended:_ New file at repo root. `## 0.2.0 - <date>` entry lists everything landed since 0.1.0, grouped under `Added` (router groups, OpenAPI + Scalar UI, benchmarks, MetricsObserver, pre-routing middleware, RequestId / Timeout / Recover middleware, Path / Query / RequestId extractors, graceful shutdown, PATCH / OPTIONS methods, body-read timeout, TLS, SSE, WebSocket, ergonomic re-exports), `Changed` (consolidated openapi module; `body::stream` helper), and `Deferred` (static files → 0.2.x, Tower adapter / HTTP/2 / workspace split → 0.3). Follows Keep-a-Changelog conventions. Verified by manual read-through.

**Step 27 — Update `docs/architecture.md`.**
_Extended:_ Add four subsections: "TLS wiring" (where the acceptor lives in the accept-loop flow), "Streaming response bodies" (the `body::stream` primitive and its relationship to `BoxBody`), "WebSocket upgrade flow" (extractor → 101 response → detached task), and "Graceful shutdown and upgraded connections" (the explicit carve-out). Refresh the dependency table. Verified by manual read-through.

**Step 28 — Update `README.md`.**
_Extended:_ Three new code snippets (minimal TLS setup with `from_pem_files`, SSE counter endpoint with keep-alive, WebSocket echo handler). Update the feature-flag table to reflect all live flags. Bump the install snippet to `flowgate = "0.2.0"`. Verified by manual read-through and rendering the README via `cargo doc --open` if README is linked.

**Step 29 — `cargo package` dry-run.**
_Extended:_ Run `cargo package --allow-dirty` and confirm `target/package/flowgate-0.2.0.crate` is produced with no warnings about missing metadata (`description`, `license`, `repository`, `readme` all present in `Cargo.toml`). Verify the included file list does not leak anything unintended (no `target/`, no local scratch files). This is the last check before `cargo publish` — which is deliberately left as a manual step outside this plan.

## Critical Files (for execution reference)

- `src/server.rs:170-220` — TCP accept loop; TLS branch goes inside the spawned connection task
- `src/body.rs` — add `stream()` alongside `full()`/`empty()`; signature exposes `Bytes`, not `Frame`
- `src/lib.rs:1-49` — module declarations and re-exports; update for new modules, drop `openapi_stub`
- `src/config.rs:7-30` — `ServerConfig` struct; add `tls` field
- `src/error.rs` — add `BoxError` type alias
- `src/response.rs` — reference for `IntoResponse` pattern; `Sse` follows this shape
- `src/extract/mod.rs:15-32` — `FromRequestParts` pattern; `WebSocketUpgrade` follows this
- `src/middleware/mod.rs` — reference for `BoxFuture` pattern used by WS callback spawn
- `src/openapi_stub.rs` — delete; fold into `src/openapi/mod.rs`
- `Cargo.toml` — deps + feature wiring + version bump

## Verification

1. `cargo build --all-features` — clean compile
2. `cargo test` (default features) — all existing tests still pass; SSE test passes
3. `cargo test --all-features` — TLS + WS + SSE integration tests pass, including PKCS#8/RSA/SEC1 key variants and compound `Connection` header
4. `cargo clippy --all-targets --all-features -- -D warnings` — zero warnings
5. `cargo doc --no-deps --all-features` — no broken links
6. `cargo run --example hello` — still works, unchanged
7. `cargo run --example groups --features openapi` — still works, unchanged
8. `cargo run --example tls --features tls` — `curl -k https://localhost:8443/` returns expected body
9. `cargo run --example sse` — `curl -N http://localhost:8080/events` streams `data:` lines and `:` heartbeat lines at the configured interval
10. `cargo run --example ws_echo --features ws` — `websocat ws://localhost:8080/ws` round-trips a frame; retry with `Connection: keep-alive, Upgrade` also succeeds
11. `cargo bench` — allocation/latency baselines unchanged within noise (regression check on the hot path)
12. `cargo package --allow-dirty` — metadata validates; `target/package/flowgate-0.2.0.crate` is produced

## Out-of-Scope Reminders

- No static file serving (explicit 0.2.x follow-up)
- No Tower adapter
- No HTTP/2 (ALPN only `http/1.1`)
- No workspace split
- No proc macros
- No route-level TLS (TLS is server-wide)
- No waiting for upgraded (WebSocket) connections on shutdown
