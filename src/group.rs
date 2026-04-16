use std::sync::Arc;

use http::Method;

use crate::app::RawRoute;
use crate::error::RouteError;
use crate::handler::{into_endpoint, Handler};
use crate::middleware::Middleware;

/// Route group — accumulates routes, middleware, tags, and subgroups
/// under a shared path prefix.
///
/// Groups are flattened into `RawRoute`s at finalization. This means:
/// - Path prefixes are concatenated (parent + child + route)
/// - Middleware stacks are merged (parent group → child group → route)
/// - Tags are inherited (parent group tags ++ child group tags ++ route tags)
///
/// Groups carry only route-local middleware. App-level middleware merging
/// happens in `App::finalize()`, so builder order does not matter.
///
/// # Example
///
/// ```ignore
/// let api = Group::new("/api")
///     .tag("api")
///     .layer(TimeoutMiddleware::new(Duration::from_secs(10)))
///     .get("/users", list_users)?
///     .group(
///         Group::new("/admin")
///             .tag("admin")
///             .get("/stats", admin_stats)?,
///     );
///
/// let app = App::new().group(api);
/// ```
pub struct Group<S> {
    prefix: String,
    tags: Vec<String>,
    middleware: Vec<Arc<dyn Middleware<S>>>,
    routes: Vec<RawRoute<S>>,
    subgroups: Vec<Group<S>>,
}

impl<S: Send + Sync + 'static> Group<S> {
    /// Create a new route group with the given path prefix.
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            tags: Vec::new(),
            middleware: Vec::new(),
            routes: Vec::new(),
            subgroups: Vec::new(),
        }
    }

    /// Add a tag to this group. Routes in this group (and nested subgroups)
    /// inherit the tag.
    pub fn tag(mut self, tag: &str) -> Self {
        self.tags.push(tag.to_owned());
        self
    }

    /// Add middleware to this group. Routes in this group (and nested subgroups)
    /// inherit the middleware.
    pub fn layer<M: Middleware<S> + 'static>(mut self, middleware: M) -> Self {
        self.middleware.push(Arc::new(middleware));
        self
    }

    /// Register a route with a specific HTTP method.
    pub fn route<H, T>(mut self, method: Method, path: &str, handler: H) -> Result<Self, RouteError>
    where
        H: Handler<T, S> + Send + Sync + 'static,
        T: Send + 'static,
    {
        validate_group_route_path(path)?;
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

    /// Nest a subgroup. The subgroup inherits this group's prefix,
    /// middleware, and tags.
    pub fn group(mut self, subgroup: Group<S>) -> Self {
        self.subgroups.push(subgroup);
        self
    }

    /// Flatten this group (and all subgroups recursively) into raw routes
    /// with fully-resolved prefixes, merged route-local middleware, and
    /// inherited tags.
    pub(crate) fn flatten(self) -> Vec<RawRoute<S>> {
        self.flatten_with(&[], &[])
    }

    /// Internal recursive flattener.
    fn flatten_with(
        self,
        parent_middleware: &[Arc<dyn Middleware<S>>],
        parent_tags: &[String],
    ) -> Vec<RawRoute<S>> {
        let mut result = Vec::new();

        // Merge: parent middleware ++ this group's middleware
        let group_mw: Vec<Arc<dyn Middleware<S>>> = parent_middleware
            .iter()
            .chain(self.middleware.iter())
            .cloned()
            .collect();

        // Merge: parent tags ++ this group's tags
        let group_tags: Vec<String> = parent_tags
            .iter()
            .chain(self.tags.iter())
            .cloned()
            .collect();

        // Flatten own routes
        for mut route in self.routes {
            route.path = normalize_group_path(&self.prefix, &route.path);

            // Route middleware = group middleware ++ route's own middleware
            let mut merged_mw = group_mw.clone();
            merged_mw.append(&mut route.route_middleware);
            route.route_middleware = merged_mw;

            // Route tags = group tags (route-level tags not yet supported, but trivially addable)
            // For now, routes inherit group tags only.
            // We don't overwrite existing route tags — just prepend group tags.
            let mut merged_tags = group_tags.clone();
            merged_tags.append(&mut route.tags);
            route.tags = merged_tags;

            result.push(route);
        }

        // Recursively flatten subgroups
        for mut subgroup in self.subgroups {
            // Prepend this group's prefix to the subgroup's prefix
            subgroup.prefix = normalize_group_path(&self.prefix, &subgroup.prefix);
            result.extend(subgroup.flatten_with(&group_mw, &group_tags));
        }

        result
    }
}

/// Validate a group route path — must start with '/' or be empty.
///
/// Empty paths are valid for group routes (the group prefix provides the path).
fn validate_group_route_path(path: &str) -> Result<(), RouteError> {
    if path.is_empty() || path.starts_with('/') {
        Ok(())
    } else {
        Err(RouteError(format!(
            "route registration failed: path must start with '/' or be empty: {path}"
        )))
    }
}

/// Normalize the concatenation of a group prefix and a route/subgroup path.
///
/// Handles trailing/leading slash mismatches:
/// - `"/api" + "/users"` → `"/api/users"`
/// - `"/api/" + "/users"` → `"/api/users"`
/// - `"" + "/users"` → `"/users"`
/// - `"/api" + ""` → `"/api"`
fn normalize_group_path(prefix: &str, path: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    let path = path.trim_start_matches('/');

    if prefix.is_empty() {
        return format!("/{path}");
    }
    if path.is_empty() {
        return prefix.to_owned();
    }
    format!("{prefix}/{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_basic() {
        assert_eq!(normalize_group_path("/api", "/users"), "/api/users");
    }

    #[test]
    fn normalize_path_trailing_slash() {
        assert_eq!(normalize_group_path("/api/", "/users"), "/api/users");
    }

    #[test]
    fn normalize_path_empty_prefix() {
        assert_eq!(normalize_group_path("", "/users"), "/users");
    }

    #[test]
    fn normalize_path_empty_path() {
        assert_eq!(normalize_group_path("/api", ""), "/api");
    }

    #[test]
    fn normalize_path_both_slashes() {
        assert_eq!(normalize_group_path("/api/", "/users/"), "/api/users/");
    }

    #[test]
    fn normalize_path_double_empty() {
        assert_eq!(normalize_group_path("", ""), "/");
    }
}
