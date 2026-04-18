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

### Phase 1 — TLS (`tls` feature)
1. Add `BoxError` type alias in `src/error.rs`
2. Add `TlsConfig` + `TlsError` in `src/tls.rs`. `from_pem_files(cert, key)` reads both paths, uses `rustls_pemfile::certs` and `rustls_pemfile::read_all` (iterator) to locate the first PKCS#8 / RSA / SEC1 private key, returns specific `TlsError` variants on failure
3. Add `tls: Option<TlsConfig>` to `ServerConfig`, builder method
4. In `src/server.rs`, inside the spawned per-connection task (not the accept task): if `tls` is `Some`, `TlsAcceptor::from(rustls_cfg.clone()).accept(stream).await`; on error, `tracing::warn!` + drop
5. Wrap the resulting TLS stream in `TokioIo::new(..)` and feed hyper as usual
6. Configure ALPN on the rustls `ServerConfig`: `["http/1.1"]`
7. `examples/tls.rs` generates a self-signed cert at startup via `rcgen`, wires `TlsConfig::from_rustls`
8. `tests/integration.rs::tls_round_trip_https` + `tls_accepts_pkcs8_and_rsa_keys`

### Phase 2 — SSE (always on)
9. `src/body.rs::stream()` — signature as spec'd above; wraps `StreamBody::new(mapped_stream).boxed()`
10. `src/sse.rs` — `Event` + builder; `Sse<S>` with `{ stream, keep_alive: Option<Duration> }`; `Sse::new(stream)`, `Sse::keep_alive(Duration)`; `IntoResponse` that serializes events to `Bytes` and, if `keep_alive` is `Some`, interleaves `:\n\n` comment frames from a `tokio::time::interval` using `futures_util::stream::select`
11. `examples/sse.rs` — counter emitting every second via `tokio::time::interval`, keep-alive `15s`
12. `tests/integration.rs::sse_stream_emits_events`

### Phase 3 — WebSocket (`ws` feature)
13. `src/ws.rs` — `WebSocketUpgrade` extractor: validate `Upgrade` token-aware (must contain `websocket`), `Connection` token-aware (must contain `upgrade`), `Sec-WebSocket-Version == 13`, `Sec-WebSocket-Key` decodes to 16 bytes; stash `OnUpgrade` from `parts.extensions`
14. `Sec-WebSocket-Accept`: SHA-1 of `<Key>` + `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`, base64-encoded
15. `WebSocketUpgrade::on_upgrade(callback)` — builds 101 Response with `Upgrade: websocket`, `Connection: upgrade`, and `Sec-WebSocket-Accept`; spawns a **detached** task that awaits `OnUpgrade` → `WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None)` → user callback. Explicitly document that this task is not tracked by `ServerHandle::shutdown`
16. `WebSocket` wrapper with `send` / `recv` / `close`; re-export `Message` from tungstenite
17. Update `ServerHandle::shutdown` rustdoc: "Does not wait for WebSocket or other upgraded connections; those run as detached tasks until they complete on their own."
18. `examples/ws_echo.rs`
19. `tests/integration.rs::ws_echo_round_trip` + `ws_accepts_compound_connection_header`

### Phase 4 — Arch polish
20. Delete `src/openapi_stub.rs`; fold into `src/openapi/mod.rs`; update `lib.rs`
21. Add ergonomic re-exports (`http::{Method, StatusCode, header}`, `bytes::Bytes`, `BoxError`) to `lib.rs`
22. Doc-comment sweep (flagged items only — don't touch what's already documented)
23. `cargo clippy --all-targets --all-features -- -D warnings` — fix any new warnings
24. `cargo doc --no-deps --all-features` — check for broken intra-doc links, fix

### Phase 5 — Release prep
25. Bump `Cargo.toml` version to `0.2.0`
26. Write `CHANGELOG.md` (`0.2.0` entry + "Deferred to 0.2.x: static files" + "Deferred to 0.3: Tower, HTTP/2, workspace split")
27. Update `docs/architecture.md` — TLS wiring, SSE streaming primitive, WebSocket upgrade flow, upgraded-connection shutdown semantics, updated dependency table
28. Update `README.md` — 3 new code snippets (TLS, SSE, WebSocket); feature-flag table refresh; install snippet → `flowgate = "0.2.0"`
29. `cargo package --allow-dirty` dry-run to catch metadata issues

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
