# Performance Baseline

This document establishes the reference numbers future changes compare against. The figures here are **delta-tracking tools** — they describe how Flowgate behaves relative to itself on a specific workstation, not absolute throughput claims.

## Why this document exists

Performance work without a baseline is storytelling. This page captures:

1. The harness setup (so new measurements are repeatable).
2. Caveats (so the numbers aren't over-read).
3. The per-fixture allocation and latency matrix as of the baseline date.
4. Cumulative wins from the initial optimization pass (PR1 – PR3).

When a future change lands, the expectation is: re-run the harness, check the matrix diff, and record any intentional regression with a justification.

## How to run

```bash
cargo bench --bench dispatch --features openapi
```

The harness prints two extra lines per fixture before criterion's stats:

```
[size]  dispatch/empty_get       5 bytes (response payload)
[alloc] dispatch/empty_get      26.00 allocs/round-trip  (N=2000, total=52000)
```

`[alloc]` comes from a `CountingAllocator` installed as `#[global_allocator]` inside the bench crate. It wraps `System` with `AtomicUsize` counters on `alloc` / `dealloc` / `realloc` — realloc is overridden explicitly so growth isn't double-counted. The count reported is `(alloc_end − alloc_start) / N` over a 2000-iteration run, so background noise (one-time connection setup, warmup allocations) does not dominate the per-request figure.

## Setup

- **Harness**: `criterion` with `iter_custom`, `Runtime::block_on` for async bench bodies.
- **Transport**: in-process HTTP/1.1 over loopback TCP (`127.0.0.1:0`). The server is driven by `flowgate::server::serve_with_listener`; the client side is `hyper::client::conn::http1::handshake` reusing a single keep-alive connection.
- **Why loopback TCP rather than pure in-memory**: `hyper::body::Incoming` cannot be constructed without hyper's connection machinery, and decoupling the body type to allow a pure in-memory transport would be a disproportionate refactor. Loopback keeps the measurement end-to-end (including hyper's parser and framing) while staying inside a single process.
- **Runtime**: `tokio` current-thread, matching the Flowgate default.
- **Config**: `ServerConfig::default()` with tracing init disabled.

## Caveats

These numbers are a tool, not a headline:

- **They are machine-dependent.** Absolute microseconds on another host — especially on real embedded targets — will differ. Use them for *relative* comparison across Flowgate versions on the *same* machine.
- **They exercise a single in-process connection.** Real traffic will hit connection setup, TLS, OS scheduling, and network latency. Those layers are out of scope for this harness.
- **The alloc counter records heap calls, not bytes.** A single `format!` that grows to 4 KB counts as one event. Bytes-level accounting is not currently tracked.
- **Single-threaded, single-client.** There is no concurrency stress in this harness. That is deliberate — it isolates the framework's own cost from scheduler and contention effects. Layer 3 (external load generation) is the right tool for those questions.

## Fixture matrix

Each fixture exercises a specific path through the framework. The baseline figures below reflect the state after PR1 – PR3 have landed.

| Fixture                       | Payload (bytes) | Allocs / req | p50 latency (µs) | What it exercises                                       |
| ----------------------------- | --------------- | ------------ | ---------------- | ------------------------------------------------------- |
| `empty_get`                   | 5               | 26           | ~6.9             | Minimal GET; no extractors, no middleware               |
| `empty_get_tracing`           | 5               | 28           | ~7.1             | +`TracingMiddleware`                                    |
| `empty_get_request_id`        | 5               | 33           | ~7.5             | +`RequestIdMiddleware` (pre-routing)                    |
| `path_param`                  | 7               | 31           | ~7.1             | `/users/{id}` with `Path<u64>` extractor                |
| `json_echo`                   | 7               | 40           | ~8.1             | POST with `Json<T>` input + output                      |
| `not_found`                   | 9               | 22           | ~6.7             | 404 path (no route match)                               |
| `multi_middleware_3`          | 5               | 26           | ~7.1             | Three no-op post-routing middleware layers              |
| `realistic_stack`             | 5               | 37           | ~7.7             | Tracing + RequestId + Timeout + path extractor combined |
| `openapi_json`                | 261             | 26           | ~7.1             | GET `/openapi.json` on a tiny spec                      |
| `openapi_json_large`          | 52,004          | 26           | ~10.0            | GET `/openapi.json` on a 25-route realistic spec        |

Notes:

- `not_found` is the cheapest path because no `RequestContext` is inserted.
- `json_echo` pays for body read + deserialize + serialize + response bytes.
- `multi_middleware_3` demonstrates that the Arc-based chain walker is essentially free — three no-op middlewares add **zero** allocations over `empty_get`. This is why no further work is planned on the middleware chain.
- `openapi_json` and `openapi_json_large` share the same alloc count: the per-request cost is independent of spec size because `spec_bytes` is an `Arc<Bytes>` (clone = refcount).

## Cumulative wins from PR1 – PR3

| Fixture                       | Pre-PR1 | Post-PR3 | Δ allocs |
| ----------------------------- | ------- | -------- | -------- |
| `empty_get`                   | 27      | 26       | −1       |
| `not_found`                   | 23      | 22       | −1       |
| `multi_middleware_3`          | 27      | 26       | −1       |
| `openapi_json`                | 29      | 26       | −3       |
| `openapi_json_large`          | *(new)* | 26       | new      |
| `empty_get_tracing`           | 29      | 28       | −1       |
| `path_param`                  | 32      | 31       | −1       |
| `empty_get_request_id`        | 35      | 33       | −2       |
| `realistic_stack`             | 39      | 37       | −2       |
| `json_echo`                   | 41      | 40       | −1       |

Every fixture in the matrix moved. The heaviest-traffic production shapes (`realistic_stack`, `openapi_json*`, `empty_get_request_id`) got the largest wins. Latency-wise, the most user-visible change is on `openapi_json_large`:

| | Before PR3 | After PR3 |
|---|---|---|
| allocs / req | 28 | 26 |
| p50 latency | 11.80 µs | 10.03 µs |

— a **~15 % improvement on a 52 KB response**, because the previous `Vec<u8>::clone()` on every request for the spec body was eliminated in favor of `Arc<Bytes>`.

### What each PR did (one line each)

- **PR1 — Router path-clone removal.** Dropped a `to_owned()` on `req.uri().path()` by letting NLL drop the matchit borrow earlier. Universally −1 alloc.
- **PR2 — RequestId canonical representation.** Collapsed the request-id to a single `HeaderValue` shared between the request extension and the response header. Generation uses `HeaderValue::from_maybe_shared(Bytes::from(String))` so the formatted buffer is moved, not copied.
- **PR3 — OpenAPI / Scalar docs Bytes reuse.** `spec_bytes` and `docs_html` are stored as `bytes::Bytes`, so handler-side clones are Arc increments rather than full buffer copies. Biggest win on large specs.

## What the harness does *not* measure

The following changes landed after PR3 and are covered by correctness tests rather than the bench, since the bench does not exercise them:

- **Body-read timeout on `Json<T>`.** Defensive 30-second default. The default wraps `collect()` in `tokio::time::timeout`, which is zero-alloc on the success path — confirmed by running the matrix post-change. Slow-loris behavior is covered by a dedicated integration test with raw TCP.
- **Metrics observer hook.** Zero observers registered = zero cost path, confirmed by re-running the matrix: all fixtures identical to pre-observer numbers. A bench fixture that registers an observer would mostly measure the observer implementation, not the framework — out of scope here.

## Future work

- **Layer 3 benchmarks**: external load generation (`wrk`, `oha`) against representative deployments. Separate harness, separate repo possibly.
- **Byte-accurate allocation counting**: if individual allocation *sizes* become relevant (e.g., for arena-based experiments), the current counter can be upgraded without changing the fixture interface.
- **Regression gate**: wire the matrix into CI with per-fixture alloc-count thresholds. Currently the gate is manual — the expectation on any PR is to re-run and diff.
