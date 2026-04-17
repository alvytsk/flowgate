use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use http::HeaderValue;

use crate::body::Request;
use crate::handler::BoxFuture;
use crate::middleware::{PreMiddleware, PreNext};

/// A unique request identifier — stored in request extensions by `RequestIdMiddleware`.
///
/// Wraps the ID as a [`HeaderValue`] internally so the same owned buffer is
/// shared between the request extension and the outgoing response header
/// (both are `Bytes`-backed — clones are `Arc` increments, not copies).
/// The middleware guarantees the inner value is valid UTF-8.
#[derive(Clone, Debug)]
pub struct RequestId(HeaderValue);

impl RequestId {
    /// Borrow the request ID as a string slice.
    pub fn as_str(&self) -> &str {
        // Invariant: `RequestIdMiddleware` only constructs `RequestId` from
        // UTF-8-validated sources (incoming header filtered by `to_str().is_ok()`
        // or an internally generated ASCII string).
        self.0.to_str().unwrap_or_default()
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Pre-routing middleware that propagates or generates `X-Request-Id` headers.
///
/// If the incoming request has a valid UTF-8 `X-Request-Id` header, its value
/// is preserved. Otherwise, a new ID is generated from the process ID (PID)
/// and a monotonic counter (no `uuid` crate — embedded-first).
///
/// The ID is:
/// 1. Stored as `RequestId` in request extensions (accessible via the `RequestId` extractor)
/// 2. Echoed in the `X-Request-Id` response header
///
/// Both sites share a single `HeaderValue` (Bytes-backed) — no duplicate
/// String allocations per request.
pub struct RequestIdMiddleware;

impl<S: Send + Sync + 'static> PreMiddleware<S> for RequestIdMiddleware {
    fn call(&self, mut req: Request, state: Arc<S>, next: PreNext<S>) -> BoxFuture {
        Box::pin(async move {
            let id_header = req
                .headers()
                .get("x-request-id")
                .filter(|v| v.to_str().is_ok())
                .cloned()
                .unwrap_or_else(generate_request_id);

            req.extensions_mut().insert(RequestId(id_header.clone()));

            let mut response = next.run(req, state).await;
            response.headers_mut().insert("x-request-id", id_header);

            response
        })
    }
}

fn generate_request_id() -> HeaderValue {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    // Format into an owned String; `Bytes::from(String)` moves the buffer
    // without copying, and `HeaderValue::from_maybe_shared` is zero-copy on
    // Bytes — so the whole path costs a single allocation.
    let formatted = format!("{pid:x}-{count:x}");
    HeaderValue::from_maybe_shared(Bytes::from(formatted))
        .expect("generated request id is valid header bytes")
}
