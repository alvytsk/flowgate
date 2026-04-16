use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1::Builder as Http1Builder;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;

use crate::app::App;
use crate::body::Request;
use crate::config::ServerConfig;
use crate::handler::BoxFuture;
use crate::middleware::{Next, PreMiddleware, PreNext};
use crate::router::Router;

/// Frozen runtime state shared across all connections.
struct RuntimeInner<S> {
    state: Arc<S>,
    router: Arc<Router<S>>,
    pre_middleware: Arc<[Arc<dyn PreMiddleware<S>>]>,
    /// Pre-compiled dispatch closure: routing + post-routing middleware + endpoint.
    /// Built once at startup so PreNext doesn't allocate a new closure per request.
    dispatch_fn: Arc<dyn Fn(Request, Arc<S>) -> BoxFuture + Send + Sync>,
    body_limit: usize,
}

/// Run the HTTP server, binding to the address in `config`.
pub async fn serve<S: Send + Sync + 'static>(
    app: App<S>,
    config: ServerConfig,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(config.bind_addr()).await?;
    serve_with_listener(app, config, listener).await
}

/// Run the HTTP server on a pre-bound `TcpListener`.
///
/// Useful for tests that need to bind a random port without a race window.
pub async fn serve_with_listener<S: Send + Sync + 'static>(
    app: App<S>,
    config: ServerConfig,
    listener: TcpListener,
) -> std::io::Result<()> {
    #[cfg(feature = "tracing-fmt")]
    if config.enable_default_tracing {
        use tracing_subscriber::EnvFilter;
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .try_init()
            .ok();
    }

    // Finalize: flatten groups, merge middleware, build matchit router.
    let finalized = app
        .finalize(config.json_body_limit)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;

    // Build the dispatch closure once — captures router, body_limit, state.
    // PreNext uses this to call into routing after pre-middleware completes.
    let router = finalized.router;
    let body_limit = finalized.body_limit;
    // We need to share the router across the dispatch closure and RuntimeInner.
    // Wrap it in an Arc so both can hold a reference.
    let router = Arc::new(router);
    let router_for_dispatch = router.clone();

    let dispatch_fn: Arc<dyn Fn(Request, Arc<S>) -> BoxFuture + Send + Sync> =
        Arc::new(move |req, state| {
            let router = router_for_dispatch.clone();
            Box::pin(async move { dispatch_request(&router, body_limit, req, state).await })
        });

    let inner = Arc::new(RuntimeInner {
        state: finalized.state,
        router,
        pre_middleware: finalized.pre_middleware,
        dispatch_fn,
        body_limit,
    });

    let addr = listener.local_addr()?;

    // Startup banner
    if let Some(meta) = &finalized.meta {
        tracing::info!("{} v{}", meta.title, meta.version);
    }
    tracing::info!("listening on {addr}");
    for entry in &finalized.manifest {
        tracing::info!("  {} {}", entry.method, entry.path);
    }
    #[cfg(feature = "openapi")]
    if finalized.openapi_enabled {
        tracing::info!("  docs: http://{addr}/docs");
    }

    // Explicit HTTP/1-only builder — matches the embedded-first, HTTP/1-only plan.
    let mut builder = Http1Builder::new();
    builder.keep_alive(config.keep_alive);

    // Wire TokioTimer unconditionally — required for any timeout support.
    builder.timer(TokioTimer::new());

    if let Some(timeout) = config.header_read_timeout {
        builder.header_read_timeout(timeout);
    }

    if let Some(max) = config.max_headers {
        builder.max_headers(max);
    }

    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("accept error: {e}");
                continue;
            }
        };
        let inner = inner.clone();
        let builder = builder.clone();

        tokio::spawn(async move {
            let io = TokioIo::new(stream);

            let service = service_fn(move |req: http::Request<Incoming>| {
                let inner = inner.clone();
                async move {
                    let response = handle_request(inner, req).await;
                    Ok::<_, std::convert::Infallible>(response)
                }
            });

            if let Err(err) = builder.serve_connection(io, service).await {
                let msg = err.to_string();
                if msg.contains("timeout") {
                    tracing::debug!("connection timeout from {peer_addr}: {err}");
                } else {
                    tracing::warn!("connection error from {peer_addr}: {err}");
                }
            }
        });
    }
}

/// Entry point for request handling — runs pre-middleware then dispatch.
async fn handle_request<S: Send + Sync + 'static>(
    inner: Arc<RuntimeInner<S>>,
    req: Request,
) -> http::Response<Full<Bytes>> {
    if inner.pre_middleware.is_empty() {
        // Fast path: no pre-routing middleware, go straight to dispatch.
        dispatch_request(&inner.router, inner.body_limit, req, inner.state.clone()).await
    } else {
        let pre_next = PreNext {
            chain: inner.pre_middleware.clone(),
            index: 0,
            dispatch: inner.dispatch_fn.clone(),
        };
        pre_next.run(req, inner.state.clone()).await
    }
}

/// Route matching + post-routing middleware + endpoint dispatch.
async fn dispatch_request<S: Send + Sync + 'static>(
    router: &Router<S>,
    body_limit: usize,
    mut req: Request,
    state: Arc<S>,
) -> http::Response<Full<Bytes>> {
    let route = router.match_route(&mut req, body_limit);

    match route {
        Some(route) => {
            let next = Next {
                endpoint: route.endpoint.clone(),
                middleware: route.middleware.clone(),
                index: 0,
            };
            next.run(req, state).await
        }
        None => {
            let allowed = router.allowed_methods(req.uri().path());
            if allowed.is_empty() {
                // 404 Not Found
                let mut res = http::Response::new(Full::new(Bytes::from("not found")));
                *res.status_mut() = http::StatusCode::NOT_FOUND;
                res
            } else {
                // 405 Method Not Allowed
                let allow_value = allowed
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut res =
                    http::Response::new(Full::new(Bytes::from("method not allowed")));
                *res.status_mut() = http::StatusCode::METHOD_NOT_ALLOWED;
                res.headers_mut().insert(
                    http::header::ALLOW,
                    http::HeaderValue::from_str(&allow_value).unwrap(),
                );
                res
            }
        }
    }
}
