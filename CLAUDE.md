# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Flowgate is a Rust web framework for embedded Linux systems. Single-crate design, single-threaded tokio runtime by default, hyper 1.x underneath.

## Build & Test Commands

```bash
cargo build                        # Build the library
cargo build --all-features         # Build with ws, tls, multi-thread
cargo test                         # Run all tests (22 integration tests)
cargo test <test_name>             # Run a single test by name
cargo clippy --all-targets         # Lint (must be zero warnings)
cargo doc --no-deps                # Build docs
cargo run --example hello          # Run the demo server on :8080
```

Tests use `#[tokio::test(flavor = "current_thread")]` — match this for new tests. Round-trip tests bind random ports via `TcpListener::bind("127.0.0.1:0")`.

## Architecture

### Handler Erasure (two layers)

`Handler<T, S>` (generic, user-facing) is bridged to `Endpoint<S>` (object-safe, stored as `Arc<dyn Endpoint<S>>` in the router) via `HandlerEndpoint`. `macro_rules!` in `handler.rs` generates impls for 0-8 extractor arguments — no proc macros.

The **last** handler argument uses `FromRequest` (may consume body). All preceding arguments use `FromRequestParts` (header-only). `State<T>` implements both traits so it can appear in any position.

### Ownership boundary

Erased layer (Endpoint, Middleware, Next) passes `Arc<S>`. Extractors receive `&S` — the Handler impl dereferences `&*state`. This split avoids lifetime issues in boxed futures while keeping extractor signatures clean.

### Middleware chain

Fully owned, Arc-based: `Arc<[Arc<dyn Middleware<S>>]>`. `Next<S>` walks the chain by index, then calls the endpoint. Both `Middleware::call` and `Endpoint::call` take `Arc<S>` — uniform ownership, no `&S` vs `Arc<S>` mismatch.

### RequestContext

Inserted into request extensions by the router before dispatch. Carries `RouteParams` (owned copies from matchit) and `body_limit` (from `ServerConfig`). Extractors like `Json<T>` read the limit from here.

### Body limit enforcement

`Json<T>` wraps the body in `http_body_util::Limited`. Discriminates `LengthLimitError` (413) from transport errors (400).

### Server: TokioTimer is required

hyper panics at runtime if timeouts are configured without a timer. `server.rs` wires `TokioTimer` unconditionally — do not remove this or guard it behind a conditional.

## Feature Flags

- `tracing-fmt` (default) — tracing-subscriber setup
- `multi-thread` — enables tokio multi-threaded runtime
- `ws`, `tls` — declared but implementations deferred to v0.2

## Adding a New Extractor

1. Implement `FromRequestParts<S>` (if it only needs headers) or `FromRequest<S>` (if it consumes the body)
2. If it should work in any handler position, implement both (see `extract/state.rs`)
3. Define a rejection type in `error.rs` that implements `IntoResponse`
4. Re-export from `lib.rs`
