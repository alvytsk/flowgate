//! Dispatch bench matrix.
//!
//! Drives real HTTP/1.1 round-trips over loopback TCP against the actual
//! `serve_with_listener` server. Each fixture reports:
//!
//! 1. Allocation events per round-trip, measured by a counting global
//!    allocator over a fixed iteration count.
//! 2. Round-trip latency, measured by criterion `iter_custom`.
//!
//! Scope and caveats:
//! - Client + server run on the same current-thread tokio runtime. The
//!   counting allocator sees both sides' allocations; treat the numbers as
//!   *round-trip* cost, useful for delta tracking across fixtures and
//!   across framework changes. They are NOT a headline "Flowgate costs N
//!   allocs per request" figure.
//! - Includes loopback TCP and hyper's HTTP/1 parse/encode. A pure dispatch
//!   micro-bench would need to decouple the internal `Request<Incoming>`
//!   type from hyper — deferred framework refactor.
//! - No default tracing subscriber is installed. Fixtures that use
//!   `TracingMiddleware` still do the middleware's eager work (e.g. path
//!   clone) regardless; the subscriber being absent just means the emitted
//!   events are dropped at dispatch.

use std::alloc::{GlobalAlloc, Layout, System};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use criterion::{black_box, Criterion};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;

use flowgate::handler::BoxFuture;
use flowgate::middleware::{Next, TracingMiddleware};
use flowgate::{
    App, Json, Middleware, Path, Request, RequestIdMiddleware, ServerConfig, TimeoutMiddleware,
};

// --- Counting global allocator ---------------------------------------------

struct CountingAllocator;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCS.fetch_add(1, Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Overriding realloc so the default alloc+copy+dealloc fallback
        // (which would double-count through our counters) doesn't fire.
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn alloc_count() -> usize {
    ALLOCS.load(Ordering::Relaxed)
}

// --- Fixture harness -------------------------------------------------------

type ClientBody = Full<Bytes>;

fn make_rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn connect(addr: SocketAddr) -> http1::SendRequest<ClientBody> {
    let stream = TcpStream::connect(addr).await.expect("connect loopback");
    stream.set_nodelay(true).ok();
    let (sender, conn) = http1::handshake(TokioIo::new(stream))
        .await
        .expect("http1 handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });
    sender
}

async fn issue(
    sender: &mut http1::SendRequest<ClientBody>,
    build_req: &dyn Fn() -> http::Request<ClientBody>,
) {
    let req = build_req();
    let res = sender.send_request(req).await.expect("send_request");
    // Drain body fully so the connection is ready for the next request.
    let _ = res
        .into_body()
        .collect()
        .await
        .expect("collect response body");
}

/// Shared runner: builds server, opens keep-alive client, warms up, records
/// alloc/req, then drives a criterion latency bench. The server is torn down
/// cleanly when this function returns.
fn run_fixture(
    c: &mut Criterion,
    name: &str,
    app: App<()>,
    build_req: &(dyn Fn() -> http::Request<ClientBody> + Sync),
) {
    let rt = make_rt();

    let (addr, handle) = rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let config = ServerConfig::default().enable_default_tracing(false);
        let handle = flowgate::server::serve_with_listener(app, config, listener)
            .await
            .expect("serve");
        (addr, handle)
    });

    let mut sender = rt.block_on(connect(addr));

    // Warm up: first-use hashmap rehashes, connection steady-state buffers.
    rt.block_on(async {
        for _ in 0..64 {
            issue(&mut sender, build_req).await;
        }
    });

    // Alloc profile.
    const N_ALLOC_PROFILE: usize = 2000;
    let before = alloc_count();
    rt.block_on(async {
        for _ in 0..N_ALLOC_PROFILE {
            issue(&mut sender, build_req).await;
        }
    });
    let delta = alloc_count() - before;
    let per_req = delta as f64 / N_ALLOC_PROFILE as f64;
    eprintln!(
        "[alloc] {name:<36} {per_req:>6.2} allocs/round-trip  (N={N_ALLOC_PROFILE}, total={delta})"
    );

    // Criterion latency.
    c.bench_function(name, |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let start = Instant::now();
                for _ in 0..iters {
                    issue(&mut sender, build_req).await;
                    black_box(());
                }
                start.elapsed()
            })
        });
    });

    drop(sender);
    // Give the client's conn task a beat to finish its read half.
    rt.block_on(async { tokio::time::sleep(Duration::from_millis(5)).await });
    let _ = rt.block_on(handle.shutdown());
}

fn get(uri: &'static str) -> http::Request<ClientBody> {
    http::Request::builder()
        .uri(uri)
        .header(http::header::HOST, "bench")
        .body(Full::new(Bytes::new()))
        .expect("build GET request")
}

fn post_json(uri: &'static str, body: &'static [u8]) -> http::Request<ClientBody> {
    http::Request::builder()
        .method(http::Method::POST)
        .uri(uri)
        .header(http::header::HOST, "bench")
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from_static(body)))
        .expect("build POST request")
}

// --- No-op middleware for chain-length probing -----------------------------

struct NoopMiddleware;

impl<S: Send + Sync + 'static> Middleware<S> for NoopMiddleware {
    fn call(&self, req: Request, state: Arc<S>, next: Next<S>) -> BoxFuture {
        next.run(req, state)
    }
}

// --- Fixtures --------------------------------------------------------------

fn bench_empty_get(c: &mut Criterion) {
    let app = App::new()
        .get("/", || async { "hello" })
        .expect("register GET /");
    run_fixture(c, "dispatch/empty_get", app, &|| get("/"));
}

fn bench_empty_get_tracing(c: &mut Criterion) {
    let app = App::new()
        .get("/", || async { "hello" })
        .expect("register GET /")
        .layer(TracingMiddleware);
    run_fixture(c, "dispatch/empty_get_tracing", app, &|| get("/"));
}

fn bench_empty_get_request_id(c: &mut Criterion) {
    let app = App::new()
        .get("/", || async { "hello" })
        .expect("register GET /")
        .pre(RequestIdMiddleware);
    run_fixture(c, "dispatch/empty_get_request_id", app, &|| get("/"));
}

fn bench_path_param(c: &mut Criterion) {
    async fn handler(Path(id): Path<u64>) -> String {
        format!("user {id}")
    }
    let app = App::new()
        .get("/users/{id}", handler)
        .expect("register GET /users/{id}");
    run_fixture(c, "dispatch/path_param", app, &|| get("/users/42"));
}

fn bench_json_echo(c: &mut Criterion) {
    #[derive(Deserialize, Serialize)]
    struct Echo {
        x: i64,
    }
    async fn handler(Json(body): Json<Echo>) -> Json<Echo> {
        Json(body)
    }
    let app = App::new()
        .post("/echo", handler)
        .expect("register POST /echo");
    run_fixture(c, "dispatch/json_echo", app, &|| {
        post_json("/echo", br#"{"x":1}"#)
    });
}

fn bench_not_found(c: &mut Criterion) {
    // One registered route that won't be hit, so routing actually runs.
    let app = App::new()
        .get("/health", || async { "ok" })
        .expect("register GET /health");
    run_fixture(c, "dispatch/not_found", app, &|| get("/missing"));
}

fn bench_multi_middleware_3(c: &mut Criterion) {
    // Three layers of no-op post-routing middleware — probes chain walker
    // + Arc bookkeeping cost without pulling in tracing/timeout side effects.
    let app = App::new()
        .get("/", || async { "hello" })
        .expect("register GET /")
        .layer(NoopMiddleware)
        .layer(NoopMiddleware)
        .layer(NoopMiddleware);
    run_fixture(c, "dispatch/multi_middleware_3", app, &|| get("/"));
}

/// Realistic middleware mix often seen in production: request ID (pre),
/// tracing (post), timeout (post). Separate from the pure chain-walker probe
/// because these middleware also allocate internally.
fn bench_realistic_stack(c: &mut Criterion) {
    let app = App::new()
        .get("/", || async { "hello" })
        .expect("register GET /")
        .pre(RequestIdMiddleware)
        .layer(TracingMiddleware)
        .layer(TimeoutMiddleware::new(Duration::from_secs(5)));
    run_fixture(c, "dispatch/realistic_stack", app, &|| get("/"));
}

#[cfg(feature = "openapi")]
fn bench_openapi_json(c: &mut Criterion) {
    let app = App::new()
        .get("/health", || async { "ok" })
        .expect("register GET /health")
        .with_openapi();
    run_fixture(c, "dispatch/openapi_json", app, &|| get("/openapi.json"));
}

// --- Main ------------------------------------------------------------------

fn main() {
    eprintln!();
    eprintln!("=== flowgate dispatch bench (client + server on one runtime thread) ===");
    eprintln!();

    let mut c = Criterion::default().configure_from_args();

    bench_empty_get(&mut c);
    bench_empty_get_tracing(&mut c);
    bench_empty_get_request_id(&mut c);
    bench_path_param(&mut c);
    bench_json_echo(&mut c);
    bench_not_found(&mut c);
    bench_multi_middleware_3(&mut c);
    bench_realistic_stack(&mut c);

    #[cfg(feature = "openapi")]
    bench_openapi_json(&mut c);

    c.final_summary();
}
