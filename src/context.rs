/// Owned route parameters extracted from matchit at match time.
#[derive(Clone, Debug, Default)]
pub struct RouteParams(pub Vec<(String, String)>);

/// Framework metadata inserted into request extensions before dispatch.
#[derive(Clone, Debug)]
pub struct RequestContext {
    /// Owned route params from matchit (copied from borrowed `Params<'k,'v>`).
    pub route_params: RouteParams,
    /// Body size limit for this request (from ServerConfig).
    pub body_limit: usize,
}
