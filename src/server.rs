use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
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

/// Run the HTTP server with the given application and configuration.
pub async fn serve<S: Send + Sync + 'static>(
    app: App<S>,
    config: ServerConfig,
) -> std::io::Result<()> {
    #[cfg(feature = "tracing-fmt")]
    if config.enable_default_tracing {
        use tracing_subscriber::EnvFilter;
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .init();
    }

    let (state, router, middleware) = app.into_parts();

    let inner = Arc::new(ServerInner {
        state,
        router,
        middleware,
        body_limit: config.json_body_limit,
    });

    let bind_addr = config.bind_addr();
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!("listening on {bind_addr}");

    // Build the hyper-util HTTP server builder with explicit config.
    let mut builder = AutoBuilder::new(hyper_util::rt::TokioExecutor::new());

    // Wire TokioTimer unconditionally — required for any timeout support.
    builder.http1().timer(TokioTimer::new());

    builder.http1().keep_alive(config.keep_alive);

    if let Some(timeout) = config.header_read_timeout {
        builder.http1().header_read_timeout(timeout);
    }

    if let Some(max) = config.max_headers {
        builder.http1().max_headers(max);
    }

    loop {
        let (stream, addr) = listener.accept().await?;
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
                tracing::warn!("connection error from {addr}: {err}");
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
            // 404 Not Found
            let mut res = http::Response::new(Full::new(Bytes::from("not found")));
            *res.status_mut() = http::StatusCode::NOT_FOUND;
            res
        }
    }
}
