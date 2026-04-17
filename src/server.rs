use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hyper::body::Incoming;
use hyper::server::conn::http1::Builder as Http1Builder;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::watch;

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
    body_read_timeout: Option<Duration>,
}

/// Handle to a running server. Allows graceful shutdown.
///
/// Dropping the handle without calling [`shutdown`](ServerHandle::shutdown)
/// signals the server to stop accepting connections (best-effort drain).
#[derive(Debug)]
pub struct ServerHandle {
    shutdown_tx: watch::Sender<bool>,
    join_handle: tokio::task::JoinHandle<()>,
    local_addr: std::net::SocketAddr,
}

impl ServerHandle {
    /// Signal the server to stop accepting new connections and wait for
    /// in-flight connections to complete (with a 30-second drain timeout).
    pub async fn shutdown(self) -> std::io::Result<()> {
        let _ = self.shutdown_tx.send(true);
        self.join_handle.await.map_err(std::io::Error::other)
    }

    /// Get the local address the server is bound to.
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.local_addr
    }
}

/// Run the HTTP server, binding to the address in `config`.
///
/// Returns a [`ServerHandle`] that can be used to trigger graceful shutdown.
pub async fn serve<S: Send + Sync + 'static>(
    app: App<S>,
    config: ServerConfig,
) -> std::io::Result<ServerHandle> {
    let listener = TcpListener::bind(config.bind_addr()).await?;
    serve_with_listener(app, config, listener).await
}

/// Run the HTTP server on a pre-bound `TcpListener`.
///
/// Returns a [`ServerHandle`] that can be used to trigger graceful shutdown.
/// Useful for tests that need to bind a random port without a race window.
pub async fn serve_with_listener<S: Send + Sync + 'static>(
    app: App<S>,
    config: ServerConfig,
    listener: TcpListener,
) -> std::io::Result<ServerHandle> {
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
        .finalize(config.json_body_limit, config.body_read_timeout)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;

    // Build the dispatch closure once — captures router, body_limit, state.
    // PreNext uses this to call into routing after pre-middleware completes.
    let router = finalized.router;
    let body_limit = finalized.body_limit;
    let body_read_timeout = finalized.body_read_timeout;
    let router = Arc::new(router);
    let router_for_dispatch = router.clone();

    let dispatch_fn: Arc<dyn Fn(Request, Arc<S>) -> BoxFuture + Send + Sync> =
        Arc::new(move |req, state| {
            let router = router_for_dispatch.clone();
            Box::pin(async move {
                dispatch_request(&router, body_limit, body_read_timeout, req, state).await
            })
        });

    let inner = Arc::new(RuntimeInner {
        state: finalized.state,
        router,
        pre_middleware: finalized.pre_middleware,
        dispatch_fn,
        body_limit,
        body_read_timeout,
    });

    let local_addr = listener.local_addr()?;

    // Startup banner
    if let Some(meta) = &finalized.meta {
        tracing::info!("{} v{}", meta.title, meta.version);
    }
    tracing::info!("listening on {local_addr}");
    for entry in &finalized.manifest {
        tracing::info!("  {} {}", entry.method, entry.path);
    }
    #[cfg(feature = "openapi")]
    if finalized.openapi_enabled {
        tracing::info!("  docs: http://{local_addr}/docs");
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

    // Connection limit: optional semaphore for backpressure.
    let semaphore = config
        .max_connections
        .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));

    // Shutdown channel: send `true` to signal the accept loop to stop.
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let active_connections = Arc::new(AtomicUsize::new(0));

    let active_conns = active_connections.clone();
    let join_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            // Check connection limit (non-blocking).
                            let permit = match &semaphore {
                                Some(sem) => match sem.clone().try_acquire_owned() {
                                    Ok(p) => Some(p),
                                    Err(_) => {
                                        tracing::warn!("connection limit reached, rejecting {peer_addr}");
                                        drop(stream);
                                        continue;
                                    }
                                },
                                None => None,
                            };

                            active_conns.fetch_add(1, Ordering::Relaxed);
                            let inner = inner.clone();
                            let builder = builder.clone();
                            let active_conns = active_conns.clone();

                            tokio::spawn(async move {
                                let _permit = permit; // released on drop
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

                                active_conns.fetch_sub(1, Ordering::Relaxed);
                            });
                        }
                        Err(e) => {
                            tracing::error!("accept error: {e}");
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    break;
                }
            }
        }

        // Drain: wait for in-flight connections to complete.
        let drain_timeout = std::time::Duration::from_secs(30);
        let drain_start = tokio::time::Instant::now();
        while active_conns.load(Ordering::Relaxed) > 0 {
            if drain_start.elapsed() > drain_timeout {
                tracing::warn!(
                    "shutdown drain timeout, {} connections still active",
                    active_conns.load(Ordering::Relaxed),
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });

    Ok(ServerHandle {
        shutdown_tx,
        join_handle,
        local_addr,
    })
}

/// Entry point for request handling — runs pre-middleware then dispatch.
async fn handle_request<S: Send + Sync + 'static>(
    inner: Arc<RuntimeInner<S>>,
    req: Request,
) -> crate::body::Response {
    if inner.pre_middleware.is_empty() {
        // Fast path: no pre-routing middleware, go straight to dispatch.
        dispatch_request(
            &inner.router,
            inner.body_limit,
            inner.body_read_timeout,
            req,
            inner.state.clone(),
        )
        .await
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
    body_read_timeout: Option<Duration>,
    mut req: Request,
    state: Arc<S>,
) -> crate::body::Response {
    let is_head = *req.method() == http::Method::HEAD;

    // Try exact method match first, then HEAD→GET fallback.
    let route = router
        .match_route(&mut req, body_limit, body_read_timeout)
        .or_else(|| {
            if is_head {
                router.match_route_for_method(
                    &mut req,
                    body_limit,
                    body_read_timeout,
                    &http::Method::GET,
                )
            } else {
                None
            }
        });

    match route {
        Some(route) => {
            let next = Next {
                endpoint: route.endpoint.clone(),
                middleware: route.middleware.clone(),
                index: 0,
            };
            let mut response = next.run(req, state).await;
            if is_head {
                *response.body_mut() = crate::body::empty();
            }
            response
        }
        None => {
            let allowed = router.allowed_methods(req.uri().path());
            if allowed.is_empty() {
                // 404 Not Found
                let mut res = http::Response::new(crate::body::full("not found"));
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
                    http::Response::new(crate::body::full("method not allowed"));
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
