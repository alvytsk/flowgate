use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use http::HeaderValue;

use crate::body::Request;
use crate::handler::BoxFuture;
use crate::middleware::{PreMiddleware, PreNext};

/// A unique request identifier — stored in request extensions by `RequestIdMiddleware`.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// Pre-routing middleware that propagates or generates `X-Request-Id` headers.
///
/// If the incoming request has an `X-Request-Id` header, its value is preserved.
/// Otherwise, a new ID is generated from the process ID (PID) and a monotonic
/// counter (no `uuid` crate — embedded-first).
///
/// The ID is:
/// 1. Stored as `RequestId` in request extensions (accessible via the `RequestId` extractor)
/// 2. Echoed in the `X-Request-Id` response header
pub struct RequestIdMiddleware;

impl<S: Send + Sync + 'static> PreMiddleware<S> for RequestIdMiddleware {
    fn call(&self, mut req: Request, state: Arc<S>, next: PreNext<S>) -> BoxFuture {
        Box::pin(async move {
            let request_id = req
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_owned())
                .unwrap_or_else(generate_request_id);

            req.extensions_mut()
                .insert(RequestId(request_id.clone()));

            let mut response = next.run(req, state).await;

            if let Ok(hv) = HeaderValue::from_str(&request_id) {
                response.headers_mut().insert("x-request-id", hv);
            }

            response
        })
    }
}

fn generate_request_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("{pid:x}-{count:x}")
}
