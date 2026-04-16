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
use crate::middleware::{Middleware, Next};
use crate::router::Router;

/// Shared server state passed into each connection's service function.
struct ServerInner<S> {
    state: Arc<S>,
    router: Router<S>,
    middleware: Arc<[Arc<dyn Middleware<S>>]>,
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

    let (state, router, middleware) = app.into_parts();

    let inner = Arc::new(ServerInner {
        state,
        router,
        middleware,
        body_limit: config.json_body_limit,
    });

    let addr = listener.local_addr()?;
    tracing::info!("listening on {addr}");

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
                tracing::warn!("connection error from {peer_addr}: {err}");
            }
        });
    }
}

/// Dispatch a single request through the middleware chain and router.
async fn handle_request<S: Send + Sync + 'static>(
    inner: Arc<ServerInner<S>>,
    req: Request,
) -> http::Response<Full<Bytes>> {
    // Try to match a route.
    let mut req = req;
    let endpoint = inner.router.match_route(&mut req, inner.body_limit);

    match endpoint {
        Some(endpoint) => {
            let next = Next {
                endpoint,
                middleware: inner.middleware.clone(),
                index: 0,
            };
            next.run(req, inner.state.clone()).await
        }
        None => {
            let allowed = inner.router.allowed_methods(req.uri().path());
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
