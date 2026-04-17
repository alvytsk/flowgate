# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Flowgate is a Rust web framework for embedded Linux systems. Single-crate design, single-threaded tokio runtime by default, hyper 1.x underneath.

## Documentation

- **[README.md](README.md)** — project overview, quick start, usage examples, configuration reference
- **[docs/architecture.md](docs/architecture.md)** — layer diagram, handler erasure, ownership model, dependency table. Always update it when architecture changes.

## Build & Test Commands

```bash
cargo build                        # Build the library
cargo build --all-features         # Build with ws, tls, multi-thread, recover, openapi
cargo test                         # Run all tests
cargo test --features openapi      # Run tests including OpenAPI tests
cargo test <test_name>             # Run a single test by name
cargo clippy --all-targets         # Lint (must be zero warnings)
cargo doc --no-deps                # Build docs
cargo run --example hello          # Run the demo server on :8080
cargo run --example groups --features openapi  # Groups demo (requires openapi feature)
```

Tests use `#[tokio::test(flavor = "current_thread")]` — match this for new tests. Round-trip tests bind random ports via `TcpListener::bind("127.0.0.1:0")`.

## Architecture

### Handler Erasure (two layers)

`Handler<T, S>` (generic, user-facing) is bridged to `Endpoint<S>` (object-safe, stored as `Arc<dyn Endpoint<S>>` in the router) via `HandlerEndpoint`. `macro_rules!` in `handler.rs` generates impls for 0-8 extractor arguments — no proc macros.

The **last** handler argument uses `FromRequest` (may consume body). All preceding arguments use `FromRequestParts` (header-only). `State<T>` implements both traits so it can appear in any position.

### Ownership boundary

Erased layer (Endpoint, Middleware, Next) passes `Arc<S>`. Extractors receive `&S` — the Handler impl dereferences `&*state`. This split avoids lifetime issues in boxed futures while keeping extractor signatures clean.

### Response body

`Response` uses `BoxBody<Bytes, Infallible>` from `http-body-util`. Use `body::full()` and `body::empty()` helpers to create buffered bodies. The `BoxBody` type-erasure enables future streaming support while keeping the current API simple.

### Builder/Runtime Split

Routes, groups, and middleware are accumulated raw during building. `serve()` does initialization (finalize, build RuntimeInner), spawns the accept loop, and returns a `ServerHandle` for graceful shutdown. `finalize()` flattens groups, merges app middleware into each route, builds the matchit router, generates the OpenAPI spec, and produces the frozen runtime state. Builder method order does not affect semantics.

### Middleware chain (post-routing)

Fully owned, Arc-based: `Arc<[Arc<dyn Middleware<S>>]>`. `Next<S>` walks the chain by index, then calls the endpoint. Both `Middleware::call` and `Endpoint::call` take `Arc<S>` — uniform ownership, no `&S` vs `Arc<S>` mismatch.

### Pre-routing middleware

`PreMiddleware<S>` runs before route matching (no access to route params). `PreNext<S>` walks the pre-middleware chain, then calls a dispatch closure compiled once at startup. `RequestIdMiddleware` is the primary built-in pre-routing middleware.

### RequestContext

Inserted into request extensions by the router before dispatch. Carries `RouteParams` (owned copies from matchit) and `body_limit` (from `ServerConfig`). Extractors like `Json<T>` read the limit from here.

### Body limit enforcement

`Json<T>` wraps the body in `http_body_util::Limited`. Discriminates `LengthLimitError` (413) from transport errors (400).

### Server: TokioTimer is required

hyper panics at runtime if timeouts are configured without a timer. `server.rs` wires `TokioTimer` unconditionally — do not remove this or guard it behind a conditional.

### Route Groups

`Group<S>` carries a path prefix, middleware, and tags. Groups nest via `.group()`. Flattened at finalization with a single `normalize_group_path()` routine for slash handling. Group middleware is route-local; app middleware merging happens at finalization.

## Feature Flags

- `tracing-fmt` (default) — tracing-subscriber setup
- `multi-thread` — enables tokio multi-threaded runtime
- `openapi` — OpenAPI spec generation + Scalar docs UI at `/docs`
- `recover` — RecoverMiddleware (panic catcher, needs `futures-util`)
- `ws`, `tls` — declared but implementations deferred

## Adding a New Extractor

1. Implement `FromRequestParts<S>` (if it only needs headers) or `FromRequest<S>` (if it consumes the body)
2. If it should work in any handler position, implement both (see `extract/state.rs`)
3. Define a rejection type in `error.rs` that implements `IntoResponse`
4. Re-export from `lib.rs`


Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.
