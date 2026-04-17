use std::sync::Arc;
use std::time::Duration;

use http::Method;

use crate::error::RouteError;
use crate::group::Group;
use crate::handler::{into_endpoint, Endpoint, Handler};
use crate::middleware::{Middleware, PreMiddleware};
use crate::router::{CompiledRoute, Router};

/// A raw route captured at builder time — not yet inserted into the matchit router.
///
/// Routes are stored raw so that app-level middleware can be merged in at
/// finalization, regardless of builder method order.
pub(crate) struct RawRoute<S> {
    pub method: Method,
    pub path: String,
    pub endpoint: Arc<dyn Endpoint<S>>,
    pub route_middleware: Vec<Arc<dyn Middleware<S>>>,
    pub tags: Vec<String>,
    #[cfg(feature = "openapi")]
    pub meta: Option<crate::openapi::meta::OperationMeta>,
}

/// Application-level metadata — used for startup banner and OpenAPI info.
#[derive(Clone, Debug, Default)]
pub struct AppMeta {
    pub title: String,
    pub version: String,
    pub description: Option<String>,
}

impl AppMeta {
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            version: version.into(),
            description: None,
        }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// A route manifest entry — produced at finalization for startup banner and OpenAPI.
#[cfg_attr(not(feature = "openapi"), allow(dead_code))]
pub(crate) struct ManifestEntry {
    pub method: Method,
    pub path: String,
    pub tags: Vec<String>,
    #[cfg(feature = "openapi")]
    pub meta: Option<crate::openapi::meta::OperationMeta>,
}

/// Application builder — owns state, accumulates routes and middleware.
///
/// Routes and middleware are stored raw until finalization (called internally
/// by `serve()`). This means builder method order does not affect semantics:
/// `.layer()` added after `.get()` still applies to all routes.
pub struct App<S = ()> {
    state: Arc<S>,
    pub(crate) routes: Vec<RawRoute<S>>,
    pub(crate) groups: Vec<Group<S>>,
    pub(crate) app_middleware: Vec<Arc<dyn Middleware<S>>>,
    pub(crate) pre_middleware: Vec<Arc<dyn PreMiddleware<S>>>,
    pub(crate) meta: Option<AppMeta>,
    #[cfg(feature = "openapi")]
    pub(crate) openapi_enabled: bool,
}

impl App<()> {
    /// Create a new application with unit `()` state.
    pub fn new() -> Self {
        Self {
            state: Arc::new(()),
            routes: Vec::new(),
            groups: Vec::new(),
            app_middleware: Vec::new(),
            pre_middleware: Vec::new(),
            meta: None,
            #[cfg(feature = "openapi")]
            openapi_enabled: false,
        }
    }
}

impl<S: Send + Sync + 'static> App<S> {
    /// Create a new application with the given shared state.
    pub fn with_state(state: S) -> Self {
        Self {
            state: Arc::new(state),
            routes: Vec::new(),
            groups: Vec::new(),
            app_middleware: Vec::new(),
            pre_middleware: Vec::new(),
            meta: None,
            #[cfg(feature = "openapi")]
            openapi_enabled: false,
        }
    }

    /// Set application metadata (title, version, description).
    /// Used for startup banner and OpenAPI info.
    pub fn meta(mut self, meta: AppMeta) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Register a route with a specific HTTP method.
    pub fn route<H, T>(
        mut self,
        method: Method,
        path: &str,
        handler: H,
    ) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        validate_path(path)?;
        self.routes.push(RawRoute {
            method,
            path: path.to_owned(),
            endpoint: into_endpoint(handler),
            route_middleware: Vec::new(),
            tags: Vec::new(),
            #[cfg(feature = "openapi")]
            meta: None,
        });
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

    /// Register a PATCH route.
    pub fn patch<H, T>(self, path: &str, handler: H) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route(Method::PATCH, path, handler)
    }

    /// Register an OPTIONS route.
    pub fn options<H, T>(self, path: &str, handler: H) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route(Method::OPTIONS, path, handler)
    }

    /// Register a route with operation metadata (for OpenAPI documentation).
    ///
    /// When the `openapi` feature is disabled, the metadata is accepted
    /// but silently dropped.
    #[allow(unused_variables)]
    pub fn route_with<H, T>(
        mut self,
        method: Method,
        path: &str,
        handler: H,
        meta: crate::OperationMeta,
    ) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        validate_path(path)?;
        self.routes.push(RawRoute {
            method,
            path: path.to_owned(),
            endpoint: into_endpoint(handler),
            route_middleware: Vec::new(),
            tags: Vec::new(),
            #[cfg(feature = "openapi")]
            meta: Some(meta),
        });
        Ok(self)
    }

    /// Register a GET route with operation metadata.
    pub fn get_with<H, T>(
        self,
        path: &str,
        handler: H,
        meta: crate::OperationMeta,
    ) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route_with(Method::GET, path, handler, meta)
    }

    /// Register a POST route with operation metadata.
    pub fn post_with<H, T>(
        self,
        path: &str,
        handler: H,
        meta: crate::OperationMeta,
    ) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route_with(Method::POST, path, handler, meta)
    }

    /// Register a PUT route with operation metadata.
    pub fn put_with<H, T>(
        self,
        path: &str,
        handler: H,
        meta: crate::OperationMeta,
    ) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route_with(Method::PUT, path, handler, meta)
    }

    /// Register a DELETE route with operation metadata.
    pub fn delete_with<H, T>(
        self,
        path: &str,
        handler: H,
        meta: crate::OperationMeta,
    ) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route_with(Method::DELETE, path, handler, meta)
    }

    /// Register a PATCH route with operation metadata.
    pub fn patch_with<H, T>(
        self,
        path: &str,
        handler: H,
        meta: crate::OperationMeta,
    ) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route_with(Method::PATCH, path, handler, meta)
    }

    /// Register an OPTIONS route with operation metadata.
    pub fn options_with<H, T>(
        self,
        path: &str,
        handler: H,
        meta: crate::OperationMeta,
    ) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        self.route_with(Method::OPTIONS, path, handler, meta)
    }

    /// Add a post-routing middleware layer.
    ///
    /// Post-routing middleware runs after route matching and has access to
    /// route params via `RequestContext`. Builder order does not matter —
    /// middleware is merged at finalization.
    pub fn layer<M: Middleware<S> + 'static>(mut self, middleware: M) -> Self {
        self.app_middleware.push(Arc::new(middleware));
        self
    }

    /// Add a pre-routing middleware layer.
    ///
    /// Pre-routing middleware runs before route matching. It does not have
    /// access to route params or the matched endpoint. Useful for request IDs,
    /// tracing spans, path normalization, or auth shortcuts.
    pub fn pre<M: PreMiddleware<S> + 'static>(mut self, middleware: M) -> Self {
        self.pre_middleware.push(Arc::new(middleware));
        self
    }

    /// Add a route group. Groups carry a path prefix, middleware, and tags
    /// that are inherited by all routes and subgroups within them.
    ///
    /// Groups are stored raw and flattened at finalization. Builder order
    /// does not matter — app-level `.layer()` middleware applies to all
    /// routes including those inside groups.
    pub fn group(mut self, group: Group<S>) -> Self {
        self.groups.push(group);
        self
    }

    /// Enable OpenAPI spec generation and docs UI.
    ///
    /// This is a declarative toggle — it does NOT generate the spec immediately.
    /// Finalization (called by `serve()`) generates the spec from the final route
    /// set and injects `GET /openapi.json` and `GET /docs` routes.
    ///
    /// Returns an error at finalization if `/openapi.json` or `/docs` is already
    /// registered as a user route.
    #[cfg(feature = "openapi")]
    pub fn with_openapi(mut self) -> Self {
        self.openapi_enabled = true;
        self
    }

    /// Get a reference to the shared state.
    pub fn state(&self) -> &Arc<S> {
        &self.state
    }

    /// Compile raw routes into a finalized runtime form.
    ///
    /// Called internally by `serve()`. Merges app middleware into each route's
    /// middleware chain and inserts all routes into the matchit router.
    /// Route conflicts (e.g. `{id}` vs `{name}` on the same method+path)
    /// surface here as `RouteError`.
    pub(crate) fn finalize(
        self,
        body_limit: usize,
        body_read_timeout: Option<Duration>,
    ) -> Result<FinalizedApp<S>, RouteError> {
        // 1. Collect all routes: direct routes + flattened groups.
        let mut all_routes = self.routes;
        for group in self.groups {
            all_routes.extend(group.flatten());
        }

        // 2. Build manifest (before insertion — for display + OpenAPI).
        let manifest: Vec<ManifestEntry> = all_routes
            .iter()
            .map(|r| ManifestEntry {
                method: r.method.clone(),
                path: r.path.clone(),
                tags: r.tags.clone(),
                #[cfg(feature = "openapi")]
                meta: r.meta.clone(),
            })
            .collect();

        // 3. Merge app middleware into each route and build the matchit router.
        let app_mw: Arc<[Arc<dyn Middleware<S>>]> = self.app_middleware.into();
        let mut router = Router::new();

        for raw in all_routes {
            let merged_mw = if raw.route_middleware.is_empty() {
                app_mw.clone() // fast path: no extra allocation
            } else {
                let mut merged: Vec<_> = app_mw.iter().cloned().collect();
                merged.extend(raw.route_middleware);
                Arc::from(merged)
            };
            let compiled = Arc::new(CompiledRoute {
                endpoint: raw.endpoint,
                middleware: merged_mw,
            });
            router.insert(raw.method, &raw.path, compiled)?;
        }

        // 4. OpenAPI: generate spec and inject docs routes if enabled.
        #[cfg(feature = "openapi")]
        let openapi_enabled = self.openapi_enabled;
        #[cfg(feature = "openapi")]
        if openapi_enabled {
            register_openapi_routes(&manifest, &self.meta, &mut router)?;
        }

        let pre_mw: Arc<[Arc<dyn PreMiddleware<S>>]> = self.pre_middleware.into();

        Ok(FinalizedApp {
            state: self.state,
            router,
            pre_middleware: pre_mw,
            body_limit,
            body_read_timeout,
            manifest,
            meta: self.meta,
            #[cfg(feature = "openapi")]
            openapi_enabled,
        })
    }
}

/// The finalized runtime form of the application.
pub(crate) struct FinalizedApp<S> {
    pub state: Arc<S>,
    pub router: Router<S>,
    pub pre_middleware: Arc<[Arc<dyn PreMiddleware<S>>]>,
    pub body_limit: usize,
    pub body_read_timeout: Option<Duration>,
    pub manifest: Vec<ManifestEntry>,
    pub meta: Option<AppMeta>,
    #[cfg(feature = "openapi")]
    pub openapi_enabled: bool,
}

impl Default for App<()> {
    fn default() -> Self {
        Self::new()
    }
}

/// Basic path validation — catches obvious mistakes at builder time.
fn validate_path(path: &str) -> Result<(), RouteError> {
    if !path.starts_with('/') {
        return Err(RouteError(format!(
            "route registration failed: path must start with '/': {path}"
        )));
    }
    Ok(())
}

/// Register OpenAPI spec and docs UI routes.
///
/// Checks for path conflicts, generates the spec from the manifest,
/// and inserts `GET /openapi.json` and `GET /docs` into the router.
#[cfg(feature = "openapi")]
fn register_openapi_routes<S: Send + Sync + 'static>(
    manifest: &[ManifestEntry],
    meta: &Option<AppMeta>,
    router: &mut Router<S>,
) -> Result<(), RouteError> {
    use crate::openapi::spec::generate_spec;
    use crate::openapi::ui::scalar_html;

    for entry in manifest {
        if entry.path == "/openapi.json" || entry.path == "/docs" {
            return Err(RouteError(format!(
                "route registration failed: '{}' conflicts with OpenAPI docs route",
                entry.path
            )));
        }
    }

    let spec_json = generate_spec(meta, manifest);
    // Share the serialized spec across every `/openapi.json` request via
    // `Bytes` — clones are Arc increments, not O(n) buffer copies.
    let spec_bytes = bytes::Bytes::from(
        serde_json::to_vec_pretty(&spec_json).unwrap_or_default(),
    );

    // GET /openapi.json — serves the spec as JSON.
    let openapi_handler = move || {
        let body = spec_bytes.clone();
        async move {
            let mut res = http::Response::new(crate::body::full(body));
            res.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            );
            res
        }
    };
    let no_mw: Arc<[Arc<dyn Middleware<S>>]> = Arc::from(Vec::new());
    router.insert(
        Method::GET,
        "/openapi.json",
        Arc::new(CompiledRoute {
            endpoint: into_endpoint(openapi_handler),
            middleware: no_mw.clone(),
        }),
    )?;

    // GET /docs — serves the Scalar UI HTML. Same Bytes-sharing treatment.
    let docs_html = bytes::Bytes::from(scalar_html());
    let docs_handler = move || {
        let html = docs_html.clone();
        async move {
            let mut res = http::Response::new(crate::body::full(html));
            res.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/html; charset=utf-8"),
            );
            res
        }
    };
    router.insert(
        Method::GET,
        "/docs",
        Arc::new(CompiledRoute {
            endpoint: into_endpoint(docs_handler),
            middleware: no_mw,
        }),
    )?;

    Ok(())
}
