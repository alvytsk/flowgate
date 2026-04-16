use std::collections::HashMap;
use std::sync::Arc;

use http::Method;

use crate::body::Request;
use crate::context::{RequestContext, RouteParams};
use crate::error::RouteError;
use crate::handler::{into_endpoint, Endpoint, Handler};

/// HTTP method router backed by matchit radix tries.
pub(crate) struct Router<S> {
    routes: HashMap<Method, matchit::Router<Arc<dyn Endpoint<S>>>>,
}

impl<S: Send + Sync + 'static> Router<S> {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// Register a handler for a method + path.
    pub fn add<H, T>(&mut self, method: Method, path: &str, handler: H) -> Result<(), RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        let endpoint = into_endpoint(handler);
        self.routes
            .entry(method)
            .or_default()
            .insert(path, endpoint)
            .map_err(|e| RouteError(e.to_string()))
    }

    /// Return HTTP methods that have a route matching the given path.
    pub fn allowed_methods(&self, path: &str) -> Vec<Method> {
        self.routes
            .iter()
            .filter(|(_, router)| router.at(path).is_ok())
            .map(|(method, _)| method.clone())
            .collect()
    }

    /// Match a request and return the endpoint + inject RequestContext into extensions.
    pub fn match_route(
        &self,
        req: &mut Request,
        body_limit: usize,
    ) -> Option<Arc<dyn Endpoint<S>>> {
        let method = req.method().clone();
        let path = req.uri().path().to_owned();

        let method_router = self.routes.get(&method)?;
        let matched = method_router.at(&path).ok()?;

        let route_params = RouteParams(
            matched
                .params
                .iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        );

        req.extensions_mut().insert(RequestContext {
            route_params,
            body_limit,
        });

        Some(matched.value.clone())
    }
}
