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

### Phase 2 — SSE (always on)

**Step 8 — Add `body::stream()` helper.**
_Extended:_ In `src/body.rs`, add `pub fn stream<S, E>(s: S) -> ResponseBody where S: Stream<Item = Result<Bytes, E>> + Send + 'static, E: Into<BoxError>`. Implementation maps each item into `Result<Frame<Bytes>, BoxError>` via `Frame::data(..)` and `Into::into`, then wraps with `StreamBody::new(..).boxed()`. The public surface stays on `Bytes` — trailers and raw-frame control are not exposed in v0.2; if needed later, a `frames_stream()` companion can be added without breaking this one. Verified by a unit test that creates a vec-backed stream and reads the body back through `BodyExt::collect`.

**Step 9 — Build the SSE module (`src/sse.rs`).**
_Extended:_ Unconditional (no feature gate). `Event` is a plain struct with builder methods `data(impl Into<String>)`, `event(impl Into<String>)`, `id(impl Into<String>)`, `retry(Duration)`, and an internal `to_bytes()` that produces the wire representation (each field on its own `key: value\n` line, terminated with a blank line). `Sse<S>` holds `{ stream: S, keep_alive: Option<Duration> }`, with `Sse::new(stream)` and `Sse::keep_alive(Duration)` builder methods. The `IntoResponse` impl sets `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `X-Accel-Buffering: no`; if `keep_alive` is `Some`, it builds a second stream from `tokio::time::interval` that emits the bytes `":\n\n"` (SSE comment — ignored by clients but keeps the socket warm for proxies), and merges the two via `futures_util::stream::select`. Final body goes through `body::stream()`. Verified by step 11.

**Step 10 — Build `examples/sse.rs`.**
_Extended:_ Handler returns `Sse::new(stream).keep_alive(Duration::from_secs(15))` where `stream` is built from `tokio::time::interval(Duration::from_secs(1))` mapped to `Event::default().data(format!("tick {n}"))`. Doc-comment notes `curl -N http://localhost:8080/events` as the manual smoke test. Verified by `cargo run --example sse` + curl.

**Step 11 — Add SSE integration test.**
_Extended:_ `sse_stream_emits_events` in `tests/integration.rs`. Binds a random port, registers an `/events` handler that emits three known events and stops (finite stream), builds an HTTP client, reads the response body in chunks, and asserts: response status 200; `Content-Type: text/event-stream`; body contains three `data:` lines with the expected payloads. A second variant test uses a keep-alive interval of 50ms and confirms at least one heartbeat (`:\n\n`) arrives between events. Verified by `cargo test`.

### Phase 3 — WebSocket (`ws` feature)

**Step 12 — Scaffold `src/ws.rs` with the `WebSocketUpgrade` extractor.**
_Extended:_ Feature-gated. Add deps `sha1 = "0.10"` and `base64 = "0.22"` under the `ws` feature in `Cargo.toml`. `WebSocketUpgrade` struct holds `{ on_upgrade: hyper::upgrade::OnUpgrade, sec_accept: http::HeaderValue, sub_protocols: Vec<String> }`. `impl FromRequestParts<S> for WebSocketUpgrade`: validates the request with a shared helper `header_contains_token(headers: &HeaderMap, name: HeaderName, token: &str) -> bool` that parses comma-separated, case-insensitive tokens — used to check `Connection` contains `upgrade` and `Upgrade` contains `websocket`. Checks `Sec-WebSocket-Version == 13` and that `Sec-WebSocket-Key` base64-decodes to exactly 16 bytes. On failure, returns a `WsError` rejection that `IntoResponse`s to 400 with a clear body. Extracts `OnUpgrade` via `parts.extensions.remove::<OnUpgrade>()`. Verified by compilation plus step 16.

**Step 13 — Compute `Sec-WebSocket-Accept`.**
_Extended:_ Static GUID `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`. In the extractor, after validation, concatenate the raw `Sec-WebSocket-Key` header string + GUID, SHA-1 the bytes, base64-encode the 20-byte digest. Store the resulting `HeaderValue` in the extractor struct so `on_upgrade` can set it on the 101 response without re-reading request headers. Unit test with the canonical RFC 6455 example: `dGhlIHNhbXBsZSBub25jZQ==` → `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`.

**Step 14 — Implement `on_upgrade(callback)` returning a 101 Response.**
_Extended:_ `on_upgrade<F, Fut>(self, callback: F) -> Response where F: FnOnce(WebSocket) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static`. Builds an `http::Response::builder().status(101).header(Upgrade, "websocket").header(Connection, "upgrade").header(Sec-WebSocket-Accept, self.sec_accept)` with an empty body. Before returning, `tokio::spawn`s a **detached** task that awaits `self.on_upgrade.await`, wraps the resulting `Upgraded` via `TokioIo::new(..)`, builds a `WebSocketStream::from_raw_socket(.., Role::Server, None)`, then calls `callback(WebSocket(stream)).await`. If the upgrade future errors, `tracing::warn!` and drop. Explicitly document that this task is not tracked by the connection counter — upgraded tasks survive `ServerHandle::shutdown`. Verified by step 16.

**Step 15 — Add the `WebSocket` wrapper and `Message` re-export.**
_Extended:_ `WebSocket` is a thin newtype over `tokio_tungstenite::WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>`. Exposes `recv() -> Option<Result<Message, WsError>>`, `send(Message) -> Result<(), WsError>`, `close() -> Result<(), WsError>` as async methods, all delegating to `SinkExt`/`StreamExt` from `futures_util`. Re-export `tokio_tungstenite::tungstenite::Message` as `flowgate::Message` from `lib.rs`. Verified by the echo example compiling.

**Step 16 — Document WS shutdown carve-out on `ServerHandle::shutdown`.**
_Extended:_ Add a prominent rustdoc note on `ServerHandle::shutdown` in `src/server.rs`: "Does not wait for WebSocket or other upgraded connections — those run as detached tasks and survive shutdown. Applications that need to coordinate upgraded-connection closure should broadcast a shutdown signal via their app state." Mirror this in `docs/architecture.md`. This closes the "shutdown semantics are undefined" concern by writing the behavior down.

**Step 17 — Build `examples/ws_echo.rs`.**
_Extended:_ Registers `/ws` → handler `async fn ws_handler(ws: WebSocketUpgrade) -> Response { ws.on_upgrade(|mut socket| async move { while let Some(Ok(msg)) = socket.recv().await { if socket.send(msg).await.is_err() { break; } } }) }`. Doc-comment notes `websocat ws://localhost:8080/ws` as the manual smoke test. Verified by `cargo run --example ws_echo --features ws` + websocat.

**Step 18 — Add WS integration tests.**
_Extended:_ Two tests gated on `#[cfg(feature = "ws")]`. `ws_echo_round_trip` binds random port, runs the echo handler, connects a `tokio-tungstenite` client, sends a text frame, asserts the echo comes back. `ws_accepts_compound_connection_header` is the regression test against naive string equality: a raw HTTP request is crafted (or the client is forced) with `Connection: keep-alive, Upgrade` — the upgrade must succeed. Verified by `cargo test --features ws`.

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
