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
#[cfg(feature = "openapi")]
use flowgate::AppMeta;

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

    // One-shot probe: capture response payload size for context.
    let payload_size = rt.block_on(async {
        let res = sender
            .send_request(build_req())
            .await
            .expect("probe send_request");
        res.into_body()
            .collect()
            .await
            .expect("probe collect")
            .to_bytes()
            .len()
    });
    eprintln!("[size]  {name:<36} {payload_size:>6} bytes (response payload)");

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

/// Large-spec fixture: ~25 routes across 5 resource families with realistic
/// metadata (summaries, descriptions, tags, path/query params, request bodies,
/// response schemas). Content is static across runs so comparisons stay clean.
#[cfg(feature = "openapi")]
fn bench_openapi_json_large(c: &mut Criterion) {
    use flowgate::openapi::meta::{BodyMeta, OperationMeta, ParamMeta, SchemaObject};
    use serde_json::{json, Map as JsonMap, Value};

    async fn stub() -> &'static str {
        ""
    }

    fn obj(props: &[(&str, Value)], required: &[&str]) -> SchemaObject {
        let mut p = JsonMap::new();
        for (k, v) in props {
            p.insert((*k).to_owned(), v.clone());
        }
        SchemaObject {
            schema_type: Some("object".to_owned()),
            properties: Some(p),
            required: required.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        }
    }

    let s_str = || json!({"type": "string"});
    let s_int = || json!({"type": "integer", "format": "int64"});
    let s_num = || json!({"type": "number", "format": "float"});
    let s_bool = || json!({"type": "boolean"});
    let s_dt = || json!({"type": "string", "format": "date-time"});

    let error_schema = obj(
        &[
            ("code", s_int()),
            ("message", s_str()),
            ("details", s_str()),
        ],
        &["code", "message"],
    );

    let user = obj(
        &[
            ("id", s_int()),
            ("name", s_str()),
            ("email", s_str()),
            ("created_at", s_dt()),
            ("active", s_bool()),
        ],
        &["id", "name", "email"],
    );
    let user_list = SchemaObject {
        schema_type: Some("array".to_owned()),
        items: Some(Box::new(user.clone())),
        ..Default::default()
    };
    let create_user_body = obj(&[("name", s_str()), ("email", s_str())], &["name", "email"]);
    let update_user_body = obj(&[("name", s_str()), ("email", s_str())], &[]);

    let device = obj(
        &[
            ("id", s_int()),
            ("serial", s_str()),
            (
                "status",
                json!({"type": "string", "enum": ["online","offline","degraded"]}),
            ),
            ("firmware", s_str()),
            ("last_seen", s_dt()),
        ],
        &["id", "serial", "status"],
    );
    let device_list = SchemaObject {
        schema_type: Some("array".to_owned()),
        items: Some(Box::new(device.clone())),
        ..Default::default()
    };
    let create_device_body = obj(&[("serial", s_str()), ("firmware", s_str())], &["serial"]);
    let update_device_body = obj(
        &[
            ("firmware", s_str()),
            (
                "status",
                json!({"type": "string", "enum": ["online","offline","degraded"]}),
            ),
        ],
        &[],
    );

    let sensor = obj(
        &[
            ("id", s_int()),
            ("device_id", s_int()),
            ("kind", s_str()),
            ("unit", s_str()),
        ],
        &["id", "device_id", "kind"],
    );
    let reading = obj(
        &[
            ("sensor_id", s_int()),
            ("value", s_num()),
            ("recorded_at", s_dt()),
        ],
        &["sensor_id", "value", "recorded_at"],
    );
    let sensor_list = SchemaObject {
        schema_type: Some("array".to_owned()),
        items: Some(Box::new(sensor.clone())),
        ..Default::default()
    };
    let reading_list = SchemaObject {
        schema_type: Some("array".to_owned()),
        items: Some(Box::new(reading.clone())),
        ..Default::default()
    };

    let event = obj(
        &[
            ("id", s_int()),
            ("device_id", s_int()),
            ("kind", s_str()),
            (
                "severity",
                json!({"type": "string", "enum": ["info","warning","error"]}),
            ),
            ("message", s_str()),
            ("occurred_at", s_dt()),
            ("acknowledged", s_bool()),
        ],
        &["id", "kind", "severity", "occurred_at"],
    );
    let event_list = SchemaObject {
        schema_type: Some("array".to_owned()),
        items: Some(Box::new(event.clone())),
        ..Default::default()
    };

    let config_entry = obj(
        &[
            ("key", s_str()),
            ("value", s_str()),
            ("description", s_str()),
        ],
        &["key", "value"],
    );
    let update_config_body = obj(&[("value", s_str())], &["value"]);

    let health = obj(
        &[
            ("status", s_str()),
            ("uptime_secs", s_int()),
            ("version", s_str()),
        ],
        &["status"],
    );

    let page_param = || {
        ParamMeta::query("page")
            .description("Page number, 1-indexed")
            .schema(SchemaObject::integer())
    };
    let per_page_param = || {
        ParamMeta::query("per_page")
            .description("Results per page (max 100)")
            .schema(SchemaObject::integer())
    };
    let id_path_param = |name: &str, desc: &str| {
        ParamMeta::path(name)
            .description(desc)
            .schema(SchemaObject::integer())
    };

    // Accumulate everything into one App. Resource family helpers below.
    let mut app = App::new().meta(
        AppMeta::new("Flowgate bench API", "1.0.0")
            .description("Synthetic API used to benchmark OpenAPI spec generation and delivery."),
    );

    // --- users -----------------------------------------------------------
    app = app
        .get_with(
            "/users",
            stub,
            OperationMeta::new()
                .summary("List users")
                .description("Return a paginated list of users with optional status filter.")
                .operation_id("listUsers")
                .tag("users")
                .param(page_param())
                .param(per_page_param())
                .param(ParamMeta::query("status").description("Filter: active or inactive"))
                .response_with_schema(200, "Users page", user_list.clone())
                .response_with_schema(400, "Invalid query parameters", error_schema.clone()),
        )
        .unwrap()
        .get_with(
            "/users/{id}",
            stub,
            OperationMeta::new()
                .summary("Fetch a user by id")
                .operation_id("getUser")
                .tag("users")
                .param(id_path_param("id", "User id"))
                .response_with_schema(200, "User", user.clone())
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap()
        .post_with(
            "/users",
            stub,
            OperationMeta::new()
                .summary("Create a user")
                .operation_id("createUser")
                .tag("users")
                .request_body(BodyMeta::json(create_user_body.clone()))
                .response_with_schema(201, "Created", user.clone())
                .response_with_schema(422, "Validation failed", error_schema.clone()),
        )
        .unwrap()
        .put_with(
            "/users/{id}",
            stub,
            OperationMeta::new()
                .summary("Replace a user")
                .operation_id("replaceUser")
                .tag("users")
                .param(id_path_param("id", "User id"))
                .request_body(BodyMeta::json(create_user_body.clone()))
                .response_with_schema(200, "Updated", user.clone())
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap()
        .patch_with(
            "/users/{id}",
            stub,
            OperationMeta::new()
                .summary("Patch a user")
                .operation_id("patchUser")
                .tag("users")
                .param(id_path_param("id", "User id"))
                .request_body(BodyMeta::json(update_user_body.clone()))
                .response_with_schema(200, "Updated", user.clone())
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap()
        .delete_with(
            "/users/{id}",
            stub,
            OperationMeta::new()
                .summary("Delete a user")
                .operation_id("deleteUser")
                .tag("users")
                .param(id_path_param("id", "User id"))
                .response(204, "Deleted")
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap();

    // --- devices ---------------------------------------------------------
    app = app
        .get_with(
            "/devices",
            stub,
            OperationMeta::new()
                .summary("List devices")
                .description("Return devices with optional status filter.")
                .operation_id("listDevices")
                .tag("devices")
                .param(page_param())
                .param(per_page_param())
                .param(
                    ParamMeta::query("status")
                        .description("Filter by status")
                        .schema(SchemaObject::string()),
                )
                .response_with_schema(200, "Devices page", device_list.clone())
                .response_with_schema(400, "Invalid query parameters", error_schema.clone()),
        )
        .unwrap()
        .get_with(
            "/devices/{id}",
            stub,
            OperationMeta::new()
                .summary("Fetch a device")
                .operation_id("getDevice")
                .tag("devices")
                .param(id_path_param("id", "Device id"))
                .response_with_schema(200, "Device", device.clone())
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap()
        .post_with(
            "/devices",
            stub,
            OperationMeta::new()
                .summary("Register a device")
                .operation_id("createDevice")
                .tag("devices")
                .request_body(BodyMeta::json(create_device_body.clone()))
                .response_with_schema(201, "Created", device.clone()),
        )
        .unwrap()
        .put_with(
            "/devices/{id}",
            stub,
            OperationMeta::new()
                .summary("Update a device")
                .operation_id("updateDevice")
                .tag("devices")
                .param(id_path_param("id", "Device id"))
                .request_body(BodyMeta::json(update_device_body.clone()))
                .response_with_schema(200, "Updated", device.clone())
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap()
        .delete_with(
            "/devices/{id}",
            stub,
            OperationMeta::new()
                .summary("Decommission a device")
                .operation_id("deleteDevice")
                .tag("devices")
                .param(id_path_param("id", "Device id"))
                .response(204, "Deleted"),
        )
        .unwrap()
        .post_with(
            "/devices/{id}/ping",
            stub,
            OperationMeta::new()
                .summary("Ping a device")
                .description("Issue a liveness probe to the device and return the latency.")
                .operation_id("pingDevice")
                .tag("devices")
                .param(id_path_param("id", "Device id"))
                .response_with_schema(
                    200,
                    "Ping result",
                    obj(
                        &[("latency_ms", s_int()), ("reachable", s_bool())],
                        &["latency_ms", "reachable"],
                    ),
                )
                .response_with_schema(504, "Device unreachable", error_schema.clone()),
        )
        .unwrap();

    // --- sensors ---------------------------------------------------------
    app = app
        .get_with(
            "/sensors",
            stub,
            OperationMeta::new()
                .summary("List sensors")
                .operation_id("listSensors")
                .tag("sensors")
                .param(page_param())
                .param(per_page_param())
                .param(
                    ParamMeta::query("device_id")
                        .description("Filter by device id")
                        .schema(SchemaObject::integer()),
                )
                .response_with_schema(200, "Sensors", sensor_list.clone()),
        )
        .unwrap()
        .get_with(
            "/sensors/{id}",
            stub,
            OperationMeta::new()
                .summary("Fetch a sensor")
                .operation_id("getSensor")
                .tag("sensors")
                .param(id_path_param("id", "Sensor id"))
                .response_with_schema(200, "Sensor", sensor.clone())
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap()
        .get_with(
            "/sensors/{id}/readings",
            stub,
            OperationMeta::new()
                .summary("List sensor readings")
                .description("Return recent readings with a configurable window.")
                .operation_id("listSensorReadings")
                .tag("sensors")
                .param(id_path_param("id", "Sensor id"))
                .param(
                    ParamMeta::query("since")
                        .description("Include readings at or after this RFC3339 timestamp")
                        .schema(SchemaObject::string()),
                )
                .param(
                    ParamMeta::query("limit")
                        .description("Maximum number of readings returned")
                        .schema(SchemaObject::integer()),
                )
                .response_with_schema(200, "Readings", reading_list.clone()),
        )
        .unwrap()
        .get_with(
            "/sensors/{id}/latest",
            stub,
            OperationMeta::new()
                .summary("Latest reading")
                .operation_id("getLatestReading")
                .tag("sensors")
                .param(id_path_param("id", "Sensor id"))
                .response_with_schema(200, "Reading", reading.clone())
                .response_with_schema(404, "No readings yet", error_schema.clone()),
        )
        .unwrap();

    // --- events ----------------------------------------------------------
    app = app
        .get_with(
            "/events",
            stub,
            OperationMeta::new()
                .summary("List events")
                .description("Filter by severity, device, or time window.")
                .operation_id("listEvents")
                .tag("events")
                .param(page_param())
                .param(per_page_param())
                .param(
                    ParamMeta::query("severity")
                        .description("Filter: info|warning|error")
                        .schema(SchemaObject::string()),
                )
                .param(
                    ParamMeta::query("device_id")
                        .description("Filter by device id")
                        .schema(SchemaObject::integer()),
                )
                .response_with_schema(200, "Events", event_list.clone()),
        )
        .unwrap()
        .get_with(
            "/events/{id}",
            stub,
            OperationMeta::new()
                .summary("Fetch an event")
                .operation_id("getEvent")
                .tag("events")
                .param(id_path_param("id", "Event id"))
                .response_with_schema(200, "Event", event.clone())
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap()
        .post_with(
            "/events/{id}/acknowledge",
            stub,
            OperationMeta::new()
                .summary("Acknowledge an event")
                .operation_id("acknowledgeEvent")
                .tag("events")
                .param(id_path_param("id", "Event id"))
                .response_with_schema(200, "Acknowledged", event.clone())
                .response_with_schema(409, "Already acknowledged", error_schema.clone()),
        )
        .unwrap();

    // --- config ----------------------------------------------------------
    app = app
        .get_with(
            "/config",
            stub,
            OperationMeta::new()
                .summary("List configuration entries")
                .operation_id("listConfig")
                .tag("config")
                .response_with_schema(
                    200,
                    "Config entries",
                    SchemaObject {
                        schema_type: Some("array".to_owned()),
                        items: Some(Box::new(config_entry.clone())),
                        ..Default::default()
                    },
                ),
        )
        .unwrap()
        .get_with(
            "/config/{key}",
            stub,
            OperationMeta::new()
                .summary("Fetch a configuration entry")
                .operation_id("getConfig")
                .tag("config")
                .param(
                    ParamMeta::path("key")
                        .description("Configuration key")
                        .schema(SchemaObject::string()),
                )
                .response_with_schema(200, "Entry", config_entry.clone())
                .response_with_schema(404, "Not found", error_schema.clone()),
        )
        .unwrap()
        .put_with(
            "/config/{key}",
            stub,
            OperationMeta::new()
                .summary("Update a configuration entry")
                .operation_id("updateConfig")
                .tag("config")
                .param(
                    ParamMeta::path("key")
                        .description("Configuration key")
                        .schema(SchemaObject::string()),
                )
                .request_body(BodyMeta::json(update_config_body.clone()))
                .response_with_schema(200, "Updated", config_entry.clone())
                .response_with_schema(400, "Invalid value", error_schema.clone()),
        )
        .unwrap()
        .post_with(
            "/config/reload",
            stub,
            OperationMeta::new()
                .summary("Reload configuration from disk")
                .description("Triggers a re-read of the on-disk configuration.")
                .operation_id("reloadConfig")
                .tag("config")
                .response(202, "Reload scheduled")
                .response_with_schema(503, "Reload unavailable", error_schema.clone()),
        )
        .unwrap();

    // --- misc ------------------------------------------------------------
    app = app
        .get_with(
            "/health",
            stub,
            OperationMeta::new()
                .summary("Health probe")
                .operation_id("getHealth")
                .tag("system")
                .response_with_schema(200, "Health", health.clone())
                .response_with_schema(503, "Unhealthy", error_schema.clone()),
        )
        .unwrap()
        .get_with(
            "/version",
            stub,
            OperationMeta::new()
                .summary("Build / version info")
                .operation_id("getVersion")
                .tag("system")
                .response_with_schema(
                    200,
                    "Version",
                    obj(
                        &[
                            ("version", s_str()),
                            ("commit", s_str()),
                            ("built_at", s_dt()),
                        ],
                        &["version"],
                    ),
                ),
        )
        .unwrap();

    let app = app.with_openapi();

    run_fixture(c, "dispatch/openapi_json_large", app, &|| {
        get("/openapi.json")
    });
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
    {
        bench_openapi_json(&mut c);
        bench_openapi_json_large(&mut c);
    }

    c.final_summary();
}
