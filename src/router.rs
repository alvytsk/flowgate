use std::collections::HashMap;
use std::sync::Arc;

use http::Method;

use crate::body::Request;
use crate::context::{RequestContext, RouteParams};
use crate::error::RouteError;
use crate::handler::Endpoint;
use crate::middleware::Middleware;

/// Runtime route entry — fully resolved endpoint + merged middleware chain.
///
/// Produced during app finalization. The middleware chain is pre-merged
/// (app middleware ++ route-local middleware) so dispatch is a single walk.
pub(crate) struct CompiledRoute<S> {
    pub endpoint: Arc<dyn Endpoint<S>>,
    pub middleware: Arc<[Arc<dyn Middleware<S>>]>,
}

/// HTTP method router backed by matchit radix tries.
///
/// Holds compiled (finalized) routes. Insertion happens during app finalization,
/// not during builder-time route registration.
pub(crate) struct Router<S> {
    routes: HashMap<Method, matchit::Router<Arc<CompiledRoute<S>>>>,
}

impl<S: Send + Sync + 'static> Router<S> {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// Insert a compiled route for a method + path.
    pub fn insert(
        &mut self,
        method: Method,
        path: &str,
        route: Arc<CompiledRoute<S>>,
    ) -> Result<(), RouteError> {
        self.routes
            .entry(method)
            .or_default()
            .insert(path, route)
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

    /// Match a request and return the compiled route + inject RequestContext.
    pub fn match_route(
        &self,
        req: &mut Request,
        body_limit: usize,
    ) -> Option<Arc<CompiledRoute<S>>> {
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
