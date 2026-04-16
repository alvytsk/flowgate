# Flowgate Architecture

## Overview

Flowgate is a Rust web framework for embedded Linux systems with FastAPI-inspired ergonomics. It prioritizes low resource usage, compile-time safety, and a minimal dependency footprint.

## Layer Architecture

```
┌─────────────────────────────────┐
│         Application             │  app.rs, config.rs
│   (App builder, state, config)  │
├─────────────────────────────────┤
│          Middleware              │  middleware.rs
│   (TracingMiddleware, custom)    │
├─────────────────────────────────┤
│      Extraction / Response      │  extract/*, response.rs
│  (Json, Path, Query, State)     │
├─────────────────────────────────┤
│           Routing               │  router.rs
│   (matchit radix trie, params)  │
├─────────────────────────────────┤
│     Protocol / Transport        │  server.rs, body.rs
│  (hyper HTTP/1.1, TCP accept)   │
├─────────────────────────────────┤
│        Infrastructure           │  error.rs, context.rs
│  (Rejections, RequestContext)   │
└─────────────────────────────────┘
```

## Key Design Decisions

### Handler Erasure: Two-Layer Design

- **`Handler<T, S>`** — generic user-facing trait for type inference on extractor tuples
- **`Endpoint<S>`** — object-safe trait stored in the router as `Arc<dyn Endpoint<S>>`
- A bridge struct (`HandlerEndpoint`) converts between the two via `.clone()` + `Box::pin()`

### Ownership Model

- `Arc<S>` throughout the erased/middleware layer (Endpoint, Middleware, Next)
- `&S` for extractors — the Handler impl dereferences `&*state` before calling extractors
- `Arc<[Arc<dyn Middleware<S>>]>` for the middleware chain — cheaply cloneable

### Extractor Design

- `FromRequestParts<S>` — extracts from headers/metadata without consuming the body
- `FromRequest<S>` — extracts from the full request (may consume the body)
- Handler macro: last argument uses `FromRequest`, all preceding use `FromRequestParts`
- `State<T>` implements both traits so it can appear in any argument position
- `Path<T>` — deserializes route parameters via custom serde Deserializer; supports single values, tuples, and structs
- `Query<T>` — deserializes query string parameters via `serde_urlencoded`
- `Path<T>`, `Query<T>`, and `State<T>` all implement both traits so they work in any handler position

### Sub-State Extraction

`FromRef<S>` trait projects sub-state from the application state:
- `State<AppState>` works via identity impl (`AppState: Clone`)
- `State<Arc<Db>>` works via user-implemented `FromRef<AppState> for Arc<Db>` (cheap Arc clone)

### Body Size Limits

- `RequestContext` carries the runtime body limit from `ServerConfig`
- `Json<T>` reads the limit from request extensions
- Uses `http_body_util::Limited` with proper `LengthLimitError` discrimination
- Default: 256 KiB

### HTTP/1 Configuration

- hyper's `http1::Builder` requires explicit `TokioTimer` wiring for any timeout support
- Timer is wired unconditionally to prevent runtime panics
- Explicit knobs for keep-alive, header read timeout, max headers

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
