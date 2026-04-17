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
│      Extraction / Response      │  extract/*, response.rs
│  (Json, Path, Query, State,     │
│   RequestId)                    │
├─────────────────────────────────┤
│           Routing               │  router.rs
│   (matchit radix trie, params)  │
├─────────────────────────────────┤
│     Protocol / Transport        │  server.rs, body.rs
│  (hyper HTTP/1.1, TCP accept)   │
├─────────────────────────────────┤
│        Infrastructure           │  error.rs, context.rs
│  (Rejections, RequestContext)   │
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

## Known Limitations

### RecoverMiddleware is post-routing only

`RecoverMiddleware` implements `Middleware<S>` (post-routing). It catches panics in handlers and post-routing middleware, but **not** in pre-routing middleware or the routing/dispatch logic itself. This is intentional — matchit routing is infallible in practice, and pre-routing middleware panics are better caught by an outer `catch_unwind` at the connection level if needed.

### Response body type

`Response` uses `BoxBody<Bytes, Infallible>` — a type-erased body that supports both buffered and streaming responses. Helper functions `body::full()` and `body::empty()` create buffered bodies. Streaming producers can create `BoxBody` from any `http_body::Body` impl via `BodyExt::boxed()`.

## Dependencies

| Crate | Purpose |
|-------|---------|
| tokio | Async runtime (single-threaded default) |
| hyper 1.x | HTTP protocol implementation |
| hyper-util | Server utilities, TokioTimer, TokioIo |
| matchit | Zero-allocation radix trie router |
| serde + serde_json | JSON serialization |
| serde_urlencoded | Query string deserialization |
| tracing | Structured logging |
| http, http-body, http-body-util | HTTP types and body utilities |
| futures-util (optional) | `catch_unwind` for RecoverMiddleware (`recover` feature) |
