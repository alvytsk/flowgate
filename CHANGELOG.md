# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

This release focuses on per-request efficiency, slow-client defense, and first-class observability. Four user-facing outcomes:

### Fewer per-request allocations across the whole matrix

Every benchmarked request shape now allocates strictly less than before. The heaviest production stacks get the biggest wins:

| Request shape                | Before | After |
| ---------------------------- | :----: | :---: |
| `GET /health` (no middleware)  | 27     | 26    |
| `GET /users/{id}` (path param) | 32     | 31    |
| `GET /users/{id}` + request-id | 35     | 33    |
| Tracing + request-id + timeout | 39     | 37    |
| `POST /echo` (JSON round-trip) | 41     | 40    |
| 404 (no route matched)         | 23     | 22    |
| `GET /openapi.json`            | 29     | 26    |

The savings come from three targeted changes: the router no longer clones the request path, `RequestId` now shares a single `Bytes`-backed buffer between the request extension and the response header, and OpenAPI spec + Scalar docs bodies are stored as `bytes::Bytes` (clones become refcount increments). See [`docs/perf-baseline.md`](docs/perf-baseline.md) for the full matrix and methodology.

### Faster large OpenAPI responses

Serving `/openapi.json` on a realistic 52 KB spec is **~15 % faster at p50** (11.80 µs → 10.03 µs) and drops 2 allocations per request. The previous implementation cloned the serialized spec's `Vec<u8>` on every request; that clone is now an `Arc` increment.

Tiny specs show the same allocation saving without a visible latency win — the eliminated copy was below measurement noise at small sizes.

### Body-read timeout for `Json<T>` extraction

`ServerConfig::body_read_timeout` bounds how long `Json<T>` will wait for request-body bytes. Default: `Some(Duration::from_secs(30))`. When the timeout fires:

- Responds with **`408 Request Timeout`**.
- Sets **`Connection: close`** so the underlying keep-alive socket is dropped.
- `JsonRejection::BodyReadTimeout` is exposed as a public variant for custom error handling.

This defends single-threaded embedded deployments against slow-loris clients that would otherwise hold a worker indefinitely by trickling bytes.

Set to `None` to opt out:

```rust
ServerConfig::new().body_read_timeout(None)
```

### Metrics observer hook — zero cost when unused

New `MetricsObserver` trait and `App::observe(...)` registration. Observers receive a `RequestEvent` containing:

- the HTTP method,
- the **matched route pattern** (e.g. `/users/{id}`) — never the raw path, for bounded-cardinality metrics labels,
- the response status code,
- wall-clock duration inside the framework.

The event fires uniformly across matched routes, 404s, and 405s (the latter two with `route_pattern: None`). Multiple observers are supported and fire in registration order. With no observer registered, the dispatch path does not even read the clock — confirmed by the bench matrix, which is byte-identical before and after the change.

```rust
use flowgate::{App, MetricsObserver, RequestEvent};

struct Counter;
impl MetricsObserver for Counter {
    fn on_request(&self, event: &RequestEvent<'_>) {
        let label = event.route_pattern.unwrap_or("<unmatched>");
        // atomic increment, channel send, etc.
    }
}

let app = App::new()
    .get("/users/{id}", get_user)?
    .observe(Counter);
```

Observer callbacks are synchronous and run on the dispatch path — any I/O or expensive work should be forwarded over a channel to a background task.

### Added

- `ServerConfig::body_read_timeout: Option<Duration>` (default `Some(30s)`), with builder setter.
- `JsonRejection::BodyReadTimeout` — maps to 408 + `Connection: close`.
- `RequestContext::body_read_timeout` — threaded from config to the `Json` extractor.
- `MetricsObserver` trait and `RequestEvent<'a>` struct; re-exported from the crate root.
- `App::observe<O: MetricsObserver>(...)` builder method, supporting multiple observers.
- `docs/perf-baseline.md` — benchmark methodology, caveats, and the reference matrix for future changes.

### Changed

- `RequestId` wraps a `HeaderValue` internally (was `String`). Access the string via `RequestId::as_str()` or its `Display` impl. `.0` is no longer `pub`.
- `CompiledRoute` now carries its registered pattern as `Arc<str>` — required by the observer hook for bounded-cardinality keys.
- OpenAPI spec bytes and Scalar docs HTML are stored as `bytes::Bytes` (were `Vec<u8>` / `String`), so per-request clones are refcount increments.

### Tests

Test count grew from 66 → 72. New coverage includes:

- `Json<T>` stalled-body scenario → 408 + `Connection: close`.
- `Json<T>` happy path with the timeout configured.
- Observer captures the matched pattern (not the raw path), 404 pattern is `None`, 405 pattern is `None`, multiple observers all fire.
- Extended `ServerConfig` default/builder coverage for the new knob.

[Unreleased]: https://github.com/anthropics/flowgate/compare/v0.2.0...HEAD
