use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use bytes::Bytes;
use http::StatusCode;
use http_body_util::Full;
use futures_util::FutureExt;

use crate::body::Request;
use crate::handler::BoxFuture;
use crate::middleware::{Middleware, Next};

/// Post-routing middleware that catches panics in handlers.
///
/// Uses `AssertUnwindSafe` + `FutureExt::catch_unwind()` to recover from
/// panics in the awaited handler future. Returns 500 Internal Server Error
/// on panic and logs the event via `tracing::error!`.
///
/// **Scope:** This catches panics in the awaited handler future only.
/// It does NOT catch panics from detached `tokio::spawn` tasks.
pub struct RecoverMiddleware;

impl<S: Send + Sync + 'static> Middleware<S> for RecoverMiddleware {
    fn call(&self, req: Request, state: Arc<S>, next: Next<S>) -> BoxFuture {
        Box::pin(async move {
            match AssertUnwindSafe(next.run(req, state))
                .catch_unwind()
                .await
            {
                Ok(response) => response,
                Err(_panic) => {
                    tracing::error!("handler panicked — recovered");
                    let mut res = http::Response::new(Full::new(Bytes::from(
                        "internal server error",
                    )));
                    *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    res
                }
            }
        })
    }
}
