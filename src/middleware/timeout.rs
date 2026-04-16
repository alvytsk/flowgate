use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::StatusCode;
use http_body_util::Full;

use crate::body::Request;
use crate::handler::BoxFuture;
use crate::middleware::{Middleware, Next};

/// Post-routing middleware that enforces a per-request timeout.
///
/// Wraps the downstream handler + middleware in `tokio::time::timeout`.
/// Returns 504 Gateway Timeout if the timeout elapses.
pub struct TimeoutMiddleware {
    duration: Duration,
}

impl TimeoutMiddleware {
    pub fn new(duration: Duration) -> Self {
        Self { duration }
    }
}

impl<S: Send + Sync + 'static> Middleware<S> for TimeoutMiddleware {
    fn call(&self, req: Request, state: Arc<S>, next: Next<S>) -> BoxFuture {
        let duration = self.duration;
        Box::pin(async move {
            match tokio::time::timeout(duration, next.run(req, state)).await {
                Ok(response) => response,
                Err(_elapsed) => {
                    let mut res =
                        http::Response::new(Full::new(Bytes::from("request timeout")));
                    *res.status_mut() = StatusCode::GATEWAY_TIMEOUT;
                    res
                }
            }
        })
    }
}
