use std::sync::Arc;

use http::Method;

use crate::error::RouteError;
use crate::handler::Handler;
use crate::middleware::Middleware;
use crate::router::Router;

/// Application builder — owns state, router, and middleware stack.
pub struct App<S = ()> {
    state: Arc<S>,
    pub(crate) router: Router<S>,
    pub(crate) middleware: Vec<Arc<dyn Middleware<S>>>,
}

impl App<()> {
    /// Create a new application with unit `()` state.
    pub fn new() -> Self {
        Self {
            state: Arc::new(()),
            router: Router::new(),
            middleware: Vec::new(),
        }
    }
}

impl<S: Send + Sync + 'static> App<S> {
    /// Create a new application with the given shared state.
    pub fn with_state(state: S) -> Self {
        Self {
            state: Arc::new(state),
            router: Router::new(),
            middleware: Vec::new(),
        }
    }

    /// Register a route with a specific HTTP method.
    pub fn route<H, T>(mut self, method: Method, path: &str, handler: H) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.router.add(method, path, handler)?;
        Ok(self)
    }

    /// Register a GET route.
    pub fn get<H, T>(self, path: &str, handler: H) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route(Method::GET, path, handler)
    }

    /// Register a POST route.
    pub fn post<H, T>(self, path: &str, handler: H) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route(Method::POST, path, handler)
    }

    /// Register a PUT route.
    pub fn put<H, T>(self, path: &str, handler: H) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route(Method::PUT, path, handler)
    }

    /// Register a DELETE route.
    pub fn delete<H, T>(self, path: &str, handler: H) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route(Method::DELETE, path, handler)
    }

    /// Add a middleware layer.
    pub fn layer<M: Middleware<S> + 'static>(mut self, middleware: M) -> Self {
        self.middleware.push(Arc::new(middleware));
        self
    }

    /// Get a reference to the shared state.
    pub fn state(&self) -> &Arc<S> {
        &self.state
    }

    /// Consume the App and return its components for the server.
    #[allow(clippy::type_complexity)]
    pub(crate) fn into_parts(
        self,
    ) -> (
        Arc<S>,
        Router<S>,
        Arc<[Arc<dyn Middleware<S>>]>,
    ) {
        let middleware: Arc<[Arc<dyn Middleware<S>>]> = self.middleware.into();
        (self.state, self.router, middleware)
    }
}

impl Default for App<()> {
    fn default() -> Self {
        Self::new()
    }
}
