use std::sync::Arc;

use crate::body::{Request, Response};
use crate::handler::{BoxFuture, Endpoint};

/// Middleware trait — processes requests before they reach the handler.
pub trait Middleware<S>: Send + Sync + 'static {
    fn call(&self, req: Request, state: Arc<S>, next: Next<S>) -> BoxFuture;
}

/// Represents the rest of the middleware chain + the final endpoint.
pub struct Next<S> {
    pub(crate) endpoint: Arc<dyn Endpoint<S>>,
    pub(crate) middleware: Arc<[Arc<dyn Middleware<S>>]>,
    pub(crate) index: usize,
}

impl<S: Send + Sync + 'static> Next<S> {
    /// Run the next middleware in the chain, or the endpoint if all middleware
    /// have been executed.
    pub fn run(self, req: Request, state: Arc<S>) -> BoxFuture {
        if self.index < self.middleware.len() {
            let mw = self.middleware[self.index].clone();
            let next = Next {
                endpoint: self.endpoint.clone(),
                middleware: self.middleware.clone(),
                index: self.index + 1,
            };
            mw.call(req, state, next)
        } else {
            self.endpoint.call(req, state)
        }
    }
}

/// Built-in tracing middleware — logs request method, path, status, and duration.
pub struct TracingMiddleware;

impl<S: Send + Sync + 'static> Middleware<S> for TracingMiddleware {
    fn call(&self, req: Request, state: Arc<S>, next: Next<S>) -> BoxFuture {
        Box::pin(async move {
            let method = req.method().clone();
            let path = req.uri().path().to_owned();
            let start = std::time::Instant::now();

            tracing::info!("--> {method} {path}");

            let response: Response = next.run(req, state).await;
            let duration = start.elapsed();
            let status = response.status();

            tracing::info!("<-- {method} {path} {} {:?}", status.as_u16(), duration);

            response
        })
    }
}
