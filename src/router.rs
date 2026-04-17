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
    /// Automatically includes HEAD when GET is present.
    pub fn allowed_methods(&self, path: &str) -> Vec<Method> {
        let mut methods: Vec<Method> = self.routes
            .iter()
            .filter(|(_, router)| router.at(path).is_ok())
            .map(|(method, _)| method.clone())
            .collect();
        if methods.contains(&Method::GET) && !methods.contains(&Method::HEAD) {
            methods.push(Method::HEAD);
        }
        methods
    }

    /// Match a request and return the compiled route + inject RequestContext.
    pub fn match_route(
        &self,
        req: &mut Request,
        body_limit: usize,
    ) -> Option<Arc<CompiledRoute<S>>> {
        let method = req.method().clone();
        self.match_route_for_method(req, body_limit, &method)
    }

    /// Match a request against a specific HTTP method.
    ///
    /// Used internally by `match_route` and for HEAD→GET fallback in dispatch.
    pub(crate) fn match_route_for_method(
        &self,
        req: &mut Request,
        body_limit: usize,
        method: &Method,
    ) -> Option<Arc<CompiledRoute<S>>> {
        let method_router = self.routes.get(method)?;
        let matched = method_router.at(req.uri().path()).ok()?;

        let route_params = RouteParams(
            matched
                .params
                .iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        );
        let value = matched.value.clone();
        // `matched` (and its borrow on req.uri().path()) released here; safe
        // to take &mut req for the RequestContext insert below.

        req.extensions_mut().insert(RequestContext {
            route_params,
            body_limit,
        });

        Some(value)
    }
}
