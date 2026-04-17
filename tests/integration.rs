use bytes::Bytes;
use http::StatusCode;
use http_body_util::{BodyExt, Full};
use serde::{Deserialize, Serialize};

use flowgate::context::{RequestContext, RouteParams};
use flowgate::extract::json::Json;
use flowgate::extract::path::Path;
use flowgate::extract::query::Query;
use flowgate::extract::state::State;
use flowgate::extract::FromRef;
use flowgate::response::IntoResponse;
use flowgate::{App, Group, ServerConfig};

// --- IntoResponse ---

#[test]
fn string_into_response() {
    let res = "hello".to_string().into_response();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "text/plain; charset=utf-8"
    );
}

#[test]
fn static_str_into_response() {
    let res = "hello".into_response();
    assert_eq!(res.status(), StatusCode::OK);
}

#[test]
fn status_code_into_response() {
    let res = StatusCode::NOT_FOUND.into_response();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test]
fn status_string_tuple_into_response() {
    let res = (StatusCode::BAD_REQUEST, "oops".to_string()).into_response();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn result_ok_into_response() {
    let result: Result<String, StatusCode> = Ok("success".into());
    let res = result.into_response();
    assert_eq!(res.status(), StatusCode::OK);
}

#[test]
fn result_err_into_response() {
    let result: Result<String, StatusCode> = Err(StatusCode::INTERNAL_SERVER_ERROR);
    let res = result.into_response();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// --- Json IntoResponse ---

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct TestPayload {
    msg: String,
}

#[test]
fn json_into_response() {
    let payload = TestPayload {
        msg: "hi".to_string(),
    };
    let res = Json(payload).into_response();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "application/json"
    );
}

// --- State extraction ---

#[derive(Clone)]
struct TestState {
    name: String,
    count: u32,
}

impl FromRef<TestState> for String {
    fn from_ref(state: &TestState) -> Self {
        state.name.clone()
    }
}

impl FromRef<TestState> for u32 {
    fn from_ref(state: &TestState) -> Self {
        state.count
    }
}

#[tokio::test(flavor = "current_thread")]
async fn state_extraction() {
    use flowgate::extract::FromRequestParts;

    let state = TestState {
        name: "test".into(),
        count: 42,
    };

    let req = http::Request::builder().uri("/").body(()).unwrap();
    let (mut parts, _) = req.into_parts();

    let State(name): State<String> =
        State::from_request_parts(&mut parts, &state).await.unwrap();
    assert_eq!(name, "test");

    let State(count): State<u32> =
        State::from_request_parts(&mut parts, &state).await.unwrap();
    assert_eq!(count, 42);
}

// --- RouteParams + RequestContext ---

#[test]
fn route_params_default_is_empty() {
    let params = RouteParams::default();
    assert!(params.0.is_empty());
}

#[test]
fn route_params_clone() {
    let params = RouteParams(vec![("id".into(), "42".into())]);
    let cloned = params.clone();
    assert_eq!(cloned.0, vec![("id".to_string(), "42".to_string())]);
}

#[test]
fn request_context_clone() {
    let ctx = RequestContext {
        route_params: RouteParams(vec![("key".into(), "val".into())]),
        body_limit: 1024,
    };
    let cloned = ctx.clone();
    assert_eq!(cloned.body_limit, 1024);
    assert_eq!(cloned.route_params.0.len(), 1);
}

// --- ServerConfig ---

#[test]
fn server_config_defaults() {
    let config = ServerConfig::default();
    assert_eq!(config.host, "0.0.0.0");
    assert_eq!(config.port, 8080);
    assert_eq!(config.bind_addr(), "0.0.0.0:8080");
    assert_eq!(config.json_body_limit, 262_144);
    assert!(config.keep_alive);
    assert!(config.header_read_timeout.is_some());
    assert!(config.max_headers.is_some());
    assert!(config.enable_default_tracing);
}

#[test]
fn server_config_builder() {
    let config = ServerConfig::new()
        .addr("127.0.0.1:3000")
        .json_body_limit(1024)
        .keep_alive(false)
        .header_read_timeout(None)
        .max_headers(None)
        .enable_default_tracing(false);

    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3000);
    assert_eq!(config.bind_addr(), "127.0.0.1:3000");
    assert_eq!(config.json_body_limit, 1024);
    assert!(!config.keep_alive);
    assert!(config.header_read_timeout.is_none());
    assert!(config.max_headers.is_none());
    assert!(!config.enable_default_tracing);
}

// --- App builder ---

#[test]
fn app_default() {
    let _app = App::new();
}

#[test]
fn app_with_state() {
    let state = TestState {
        name: "test".into(),
        count: 1,
    };
    let app = App::with_state(state);
    assert_eq!(app.state().name, "test");
}

// --- Error types ---

#[test]
fn json_rejection_display() {
    use flowgate::error::JsonRejection;

    let r = JsonRejection::PayloadTooLarge;
    assert_eq!(r.to_string(), "payload too large");

    let r = JsonRejection::BodyReadError("test".into());
    assert!(r.to_string().contains("test"));
}

#[test]
fn json_rejection_into_response() {
    use flowgate::error::JsonRejection;

    let res = JsonRejection::PayloadTooLarge.into_response();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let res = JsonRejection::BodyReadError("err".into()).into_response();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// --- Path extraction ---

#[tokio::test(flavor = "current_thread")]
async fn path_single_string() {
    use flowgate::context::{RequestContext, RouteParams};
    use flowgate::extract::FromRequestParts;

    let req = http::Request::builder().uri("/users/alice").body(()).unwrap();
    let (mut parts, _) = req.into_parts();
    parts.extensions.insert(RequestContext {
        route_params: RouteParams(vec![("name".into(), "alice".into())]),
        body_limit: 262_144,
    });

    let Path(name): Path<String> = Path::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(name, "alice");
}

#[tokio::test(flavor = "current_thread")]
async fn path_single_u64() {
    use flowgate::context::{RequestContext, RouteParams};
    use flowgate::extract::FromRequestParts;

    let req = http::Request::builder().uri("/users/42").body(()).unwrap();
    let (mut parts, _) = req.into_parts();
    parts.extensions.insert(RequestContext {
        route_params: RouteParams(vec![("id".into(), "42".into())]),
        body_limit: 262_144,
    });

    let Path(id): Path<u64> = Path::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(id, 42);
}

#[tokio::test(flavor = "current_thread")]
async fn path_tuple() {
    use flowgate::context::{RequestContext, RouteParams};
    use flowgate::extract::FromRequestParts;

    let req = http::Request::builder()
        .uri("/users/7/posts/99")
        .body(())
        .unwrap();
    let (mut parts, _) = req.into_parts();
    parts.extensions.insert(RequestContext {
        route_params: RouteParams(vec![
            ("uid".into(), "7".into()),
            ("pid".into(), "99".into()),
        ]),
        body_limit: 262_144,
    });

    let Path((uid, pid)): Path<(u64, u64)> =
        Path::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(uid, 7);
    assert_eq!(pid, 99);
}

#[tokio::test(flavor = "current_thread")]
async fn path_struct() {
    use flowgate::context::{RequestContext, RouteParams};
    use flowgate::extract::FromRequestParts;

    #[derive(Deserialize)]
    struct Params {
        user_id: u64,
        slug: String,
    }

    let req = http::Request::builder()
        .uri("/users/5/posts/hello-world")
        .body(())
        .unwrap();
    let (mut parts, _) = req.into_parts();
    parts.extensions.insert(RequestContext {
        route_params: RouteParams(vec![
            ("user_id".into(), "5".into()),
            ("slug".into(), "hello-world".into()),
        ]),
        body_limit: 262_144,
    });

    let Path(p): Path<Params> = Path::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(p.user_id, 5);
    assert_eq!(p.slug, "hello-world");
}

#[tokio::test(flavor = "current_thread")]
async fn path_invalid_u64_returns_error() {
    use flowgate::context::{RequestContext, RouteParams};
    use flowgate::extract::FromRequestParts;

    let req = http::Request::builder().uri("/users/abc").body(()).unwrap();
    let (mut parts, _) = req.into_parts();
    parts.extensions.insert(RequestContext {
        route_params: RouteParams(vec![("id".into(), "abc".into())]),
        body_limit: 262_144,
    });

    let result: Result<Path<u64>, _> = Path::from_request_parts(&mut parts, &()).await;
    assert!(result.is_err());
}

// --- Query extraction ---

#[tokio::test(flavor = "current_thread")]
async fn query_struct() {
    use flowgate::extract::FromRequestParts;

    #[derive(Deserialize)]
    struct Search {
        q: String,
        page: u32,
    }

    let req = http::Request::builder()
        .uri("/search?q=rust&page=2")
        .body(())
        .unwrap();
    let (mut parts, _) = req.into_parts();

    let Query(s): Query<Search> = Query::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(s.q, "rust");
    assert_eq!(s.page, 2);
}

#[tokio::test(flavor = "current_thread")]
async fn query_missing_returns_error() {
    use flowgate::extract::FromRequestParts;

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Required {
        q: String,
    }

    let req = http::Request::builder().uri("/search").body(()).unwrap();
    let (mut parts, _) = req.into_parts();

    let result: Result<Query<Required>, _> = Query::from_request_parts(&mut parts, &()).await;
    assert!(result.is_err());
}

// --- Rejection types ---

#[test]
fn path_rejection_into_response() {
    use flowgate::error::PathRejection;

    let res = PathRejection::MissingRouteParams.into_response();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let res = PathRejection::DeserializeError("bad".into()).into_response();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn query_rejection_into_response() {
    use flowgate::error::QueryRejection;

    let res = QueryRejection::DeserializeError("bad".into()).into_response();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// --- Full server round-trip tests ---

/// Helper: spin up a Flowgate server on a random port, return the address.
async fn serve_app<S: Send + Sync + 'static>(
    app: App<S>,
) -> (std::net::SocketAddr, flowgate::ServerHandle) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = ServerConfig::new().enable_default_tracing(false);

    let handle = flowgate::server::serve_with_listener(app, config, listener)
        .await
        .unwrap();

    (addr, handle)
}

/// Make an HTTP/1.1 request and return the response.
async fn http_request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (StatusCode, String) {
    let (status, _, body) = http_request_with_headers(addr, method, path, body, &[]).await;
    (status, body)
}

/// Make an HTTP/1.1 request with extra headers and return status, response headers, and body.
async fn http_request_with_headers(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> (StatusCode, std::collections::HashMap<String, String>, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = hyper_util::rt::TokioIo::new(stream);

    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.unwrap();
    tokio::spawn(conn);

    let req_body = match body {
        Some(b) => Full::new(Bytes::from(b.to_owned())),
        None => Full::new(Bytes::new()),
    };

    let mut builder = http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("content-type", "application/json");

    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }

    let req = builder.body(req_body).unwrap();

    let res = sender.send_request(req).await.unwrap();
    let status = res.status();

    let headers: std::collections::HashMap<String, String> = res
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
        .collect();

    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

    (status, headers, body_str)
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_get_handler() {
    #[derive(Serialize)]
    struct Pong {
        ok: bool,
    }

    async fn ping() -> Json<Pong> {
        Json(Pong { ok: true })
    }

    let app = App::new().get("/ping", ping).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/ping", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["ok"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_post_json() {
    #[derive(Deserialize)]
    struct In {
        x: i32,
    }
    #[derive(Serialize)]
    struct Out {
        doubled: i32,
    }

    async fn double(Json(input): Json<In>) -> Json<Out> {
        Json(Out {
            doubled: input.x * 2,
        })
    }

    let app = App::new().post("/double", double).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "POST", "/double", Some(r#"{"x":21}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["doubled"], 42);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_404() {
    let app = App::new();
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "GET", "/missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_state_extraction() {
    #[derive(Clone)]
    struct Counter {
        value: u32,
    }

    #[derive(Serialize)]
    struct CountResponse {
        count: u32,
    }

    async fn get_count(State(counter): State<Counter>) -> Json<CountResponse> {
        Json(CountResponse {
            count: counter.value,
        })
    }

    let app = App::with_state(Counter { value: 99 }).get("/count", get_count).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/count", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["count"], 99);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_body_limit_enforced() {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Big {
        data: String,
    }

    async fn accept_big(Json(_body): Json<Big>) -> &'static str {
        "ok"
    }

    let app = App::new().post("/big", accept_big).unwrap();

    // Use a tiny body limit
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = ServerConfig::new()
        .json_body_limit(32) // 32 bytes
        .enable_default_tracing(false);

    let _handle = flowgate::server::serve_with_listener(app, config, listener)
        .await
        .unwrap();

    // Send a body larger than 32 bytes
    let big_payload = r#"{"data":"this string is definitely longer than thirty-two bytes"}"#;
    let (status, _) = http_request(addr, "POST", "/big", Some(big_payload)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_path_param() {
    #[derive(Serialize)]
    struct User {
        id: u64,
    }

    async fn get_user(Path(id): Path<u64>) -> Json<User> {
        Json(User { id })
    }

    let app = App::new().get("/users/{id}", get_user).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/users/42", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["id"], 42);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_path_param_invalid() {
    async fn get_user(Path(_id): Path<u64>) -> &'static str {
        "ok"
    }

    let app = App::new().get("/users/{id}", get_user).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "GET", "/users/not-a-number", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_path_and_json() {
    #[derive(Deserialize)]
    struct UpdateUser {
        name: String,
    }
    #[derive(Serialize)]
    struct Updated {
        id: u64,
        name: String,
    }

    async fn update_user(Path(id): Path<u64>, Json(body): Json<UpdateUser>) -> Json<Updated> {
        Json(Updated {
            id,
            name: body.name,
        })
    }

    let app = App::new().put("/users/{id}", update_user).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) =
        http_request(addr, "PUT", "/users/7", Some(r#"{"name":"alice"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["id"], 7);
    assert_eq!(parsed["name"], "alice");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_query_params() {
    #[derive(Deserialize)]
    struct Search {
        q: String,
    }
    #[derive(Serialize)]
    struct Results {
        query: String,
    }

    async fn search(Query(s): Query<Search>) -> Json<Results> {
        Json(Results { query: s.q })
    }

    let app = App::new().get("/search", search).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/search?q=flowgate", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["query"], "flowgate");
}

// --- Route registration errors ---

#[tokio::test(flavor = "current_thread")]
async fn route_conflict_returns_error() {
    async fn handler_a() -> &'static str {
        "a"
    }
    async fn handler_b() -> &'static str {
        "b"
    }

    // Route conflicts (e.g. {id} vs {name} on the same method+path) are
    // detected at finalization (serve time), not at builder time.
    let app = App::new()
        .get("/items/{id}", handler_a)
        .unwrap()
        .get("/items/{name}", handler_b)
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ServerConfig::new().enable_default_tracing(false);
    let result = flowgate::server::serve_with_listener(app, config, listener).await;

    let err = result.expect_err("expected route conflict error");
    assert!(err.to_string().contains("route"));
}

// --- HEAD auto-handling ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_head_on_get_route() {
    async fn get_handler() -> &'static str {
        "hello world"
    }

    let app = App::new().get("/page", get_handler).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, headers, body) = http_request_with_headers(addr, "HEAD", "/page", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty(), "HEAD response body must be empty");
    assert_eq!(headers.get("content-type").unwrap(), "text/plain; charset=utf-8");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_head_no_get_route() {
    async fn post_only() -> &'static str {
        "created"
    }

    let app = App::new().post("/items", post_only).unwrap();
    let (addr, _handle) = serve_app(app).await;

    // HEAD on a route with only POST should be 405
    let (status, _body) = http_request(addr, "HEAD", "/items", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_head_nonexistent_route() {
    async fn handler() -> &'static str { "ok" }

    let app = App::new().get("/exists", handler).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, _body) = http_request(addr, "HEAD", "/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_405_allow_header_includes_head() {
    async fn handler() -> &'static str { "ok" }

    let app = App::new().get("/item", handler).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, headers, _body) = http_request_with_headers(addr, "DELETE", "/item", None, &[]).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let allow = headers.get("allow").unwrap();
    assert!(allow.contains("GET"), "Allow header should contain GET");
    assert!(allow.contains("HEAD"), "Allow header should contain HEAD");
}

// --- PATCH and OPTIONS ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_patch() {
    async fn update(Json(payload): Json<TestPayload>) -> Json<TestPayload> {
        Json(payload)
    }

    let app = App::new().patch("/items", update).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "PATCH", "/items", Some(r#"{"msg":"patched"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: TestPayload = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed.msg, "patched");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_options() {
    async fn cors_preflight() -> (http::StatusCode, &'static str) {
        (StatusCode::NO_CONTENT, "")
    }

    let app = App::new().options("/resource", cors_preflight).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, _body) = http_request(addr, "OPTIONS", "/resource", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// --- 405 Method Not Allowed ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_405_method_not_allowed() {
    async fn get_only() -> &'static str {
        "ok"
    }

    let app = App::new().get("/resource", get_only).unwrap();
    let (addr, _handle) = serve_app(app).await;

    // POST to a GET-only route
    let (status, body) = http_request(addr, "POST", "/resource", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body, "method not allowed");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_405_allow_header() {
    async fn handler() -> &'static str {
        "ok"
    }

    let app = App::new()
        .get("/item", handler)
        .unwrap()
        .put("/item", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    // DELETE to a route that only has GET + PUT
    let (status, _) = http_request(addr, "DELETE", "/item", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_404_still_works() {
    async fn handler() -> &'static str {
        "ok"
    }

    let app = App::new().get("/exists", handler).unwrap();
    let (addr, _handle) = serve_app(app).await;

    // Path that matches no route at all
    let (status, _) = http_request(addr, "GET", "/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- Middleware ordering and short-circuit ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_middleware_ordering() {
    use std::sync::Mutex;
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next};

    #[derive(Clone)]
    struct Order(std::sync::Arc<Mutex<Vec<&'static str>>>);

    struct TagMiddleware(&'static str);
    impl Middleware<Order> for TagMiddleware {
        fn call(
            &self,
            req: flowgate::Request,
            state: std::sync::Arc<Order>,
            next: Next<Order>,
        ) -> BoxFuture {
            let tag = self.0;
            Box::pin(async move {
                state.0.lock().unwrap().push(tag);
                next.run(req, state).await
            })
        }
    }

    let order = Order(std::sync::Arc::new(Mutex::new(Vec::new())));

    async fn handler(State(order): State<Order>) -> &'static str {
        order.0.lock().unwrap().push("handler");
        "ok"
    }

    let app = App::with_state(order.clone())
        .layer(TagMiddleware("first"))
        .layer(TagMiddleware("second"))
        .get("/test", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "GET", "/test", None).await;
    assert_eq!(status, StatusCode::OK);

    let log = order.0.lock().unwrap();
    assert_eq!(*log, vec!["first", "second", "handler"]);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_middleware_short_circuit() {
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next};

    struct BlockMiddleware;
    impl<S: Send + Sync + 'static> Middleware<S> for BlockMiddleware {
        fn call(
            &self,
            _req: flowgate::Request,
            _state: std::sync::Arc<S>,
            _next: Next<S>,
        ) -> BoxFuture {
            Box::pin(async {
                let mut res = http::Response::new(flowgate::body::full("blocked"));
                *res.status_mut() = StatusCode::FORBIDDEN;
                res
            })
        }
    }

    async fn handler() -> &'static str {
        "should not reach"
    }

    let app = App::new()
        .layer(BlockMiddleware)
        .get("/guarded", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/guarded", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, "blocked");
}

// --- Multi-extractor handler (3 args: State + Path + Json) ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_three_extractors() {
    #[derive(Clone)]
    struct Prefix(String);

    #[derive(Deserialize)]
    struct Body {
        value: String,
    }
    #[derive(Serialize)]
    struct Resp {
        result: String,
    }

    async fn handler(
        State(prefix): State<Prefix>,
        Path(id): Path<u64>,
        Json(body): Json<Body>,
    ) -> Json<Resp> {
        Json(Resp {
            result: format!("{}-{}-{}", prefix.0, id, body.value),
        })
    }

    let app = App::with_state(Prefix("pfx".into()))
        .post("/items/{id}", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) =
        http_request(addr, "POST", "/items/5", Some(r#"{"value":"hello"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["result"], "pfx-5-hello");
}

// --- Pre-routing middleware ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_pre_middleware_ordering() {
    use std::sync::Mutex;
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next, PreMiddleware, PreNext};

    #[derive(Clone)]
    struct Order(std::sync::Arc<Mutex<Vec<&'static str>>>);

    struct PreTag(&'static str);
    impl PreMiddleware<Order> for PreTag {
        fn call(
            &self,
            req: flowgate::Request,
            state: std::sync::Arc<Order>,
            next: PreNext<Order>,
        ) -> BoxFuture {
            let tag = self.0;
            Box::pin(async move {
                state.0.lock().unwrap().push(tag);
                next.run(req, state).await
            })
        }
    }

    struct PostTag(&'static str);
    impl Middleware<Order> for PostTag {
        fn call(
            &self,
            req: flowgate::Request,
            state: std::sync::Arc<Order>,
            next: Next<Order>,
        ) -> BoxFuture {
            let tag = self.0;
            Box::pin(async move {
                state.0.lock().unwrap().push(tag);
                next.run(req, state).await
            })
        }
    }

    let order = Order(std::sync::Arc::new(Mutex::new(Vec::new())));

    async fn handler(State(order): State<Order>) -> &'static str {
        order.0.lock().unwrap().push("handler");
        "ok"
    }

    // Pre-middleware runs before post-routing middleware, regardless of builder order.
    let app = App::with_state(order.clone())
        .layer(PostTag("post"))
        .pre(PreTag("pre"))
        .get("/test", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "GET", "/test", None).await;
    assert_eq!(status, StatusCode::OK);

    let log = order.0.lock().unwrap();
    assert_eq!(*log, vec!["pre", "post", "handler"]);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_pre_middleware_short_circuit() {
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::PreMiddleware;

    struct BlockPre;
    impl<S: Send + Sync + 'static> PreMiddleware<S> for BlockPre {
        fn call(
            &self,
            _req: flowgate::Request,
            _state: std::sync::Arc<S>,
            _next: flowgate::middleware::PreNext<S>,
        ) -> BoxFuture {
            Box::pin(async {
                let mut res = http::Response::new(flowgate::body::full("pre-blocked"));
                *res.status_mut() = StatusCode::UNAUTHORIZED;
                res
            })
        }
    }

    async fn handler() -> &'static str {
        "should not reach"
    }

    let app = App::new()
        .pre(BlockPre)
        .get("/secret", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/secret", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, "pre-blocked");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_layer_after_route_still_applies() {
    // Verifies that builder order doesn't matter: .layer() after .get()
    // still applies the middleware to the route (finalization merges them).
    use std::sync::Mutex;
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next};

    #[derive(Clone)]
    struct Log(std::sync::Arc<Mutex<Vec<&'static str>>>);

    struct TagMw;
    impl Middleware<Log> for TagMw {
        fn call(
            &self,
            req: flowgate::Request,
            state: std::sync::Arc<Log>,
            next: Next<Log>,
        ) -> BoxFuture {
            Box::pin(async move {
                state.0.lock().unwrap().push("mw");
                next.run(req, state).await
            })
        }
    }

    let log = Log(std::sync::Arc::new(Mutex::new(Vec::new())));

    async fn handler(State(log): State<Log>) -> &'static str {
        log.0.lock().unwrap().push("handler");
        "ok"
    }

    // layer added AFTER route — must still apply.
    let app = App::with_state(log.clone())
        .get("/test", handler)
        .unwrap()
        .layer(TagMw);
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "GET", "/test", None).await;
    assert_eq!(status, StatusCode::OK);

    let entries = log.0.lock().unwrap();
    assert_eq!(*entries, vec!["mw", "handler"]);
}

// --- Route Groups ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_group_basic() {
    async fn health() -> &'static str {
        "ok"
    }
    async fn list_users() -> &'static str {
        "users"
    }

    let app = App::new()
        .get("/health", health)
        .unwrap()
        .group(
            Group::new("/api")
                .get("/users", list_users)
                .unwrap(),
        );
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");

    let (status, body) = http_request(addr, "GET", "/api/users", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "users");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_nested_groups() {
    async fn stats() -> &'static str {
        "stats"
    }
    async fn users() -> &'static str {
        "users"
    }

    let app = App::new()
        .group(
            Group::new("/api")
                .get("/users", users)
                .unwrap()
                .group(
                    Group::new("/admin")
                        .get("/stats", stats)
                        .unwrap(),
                ),
        );
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/api/users", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "users");

    let (status, body) = http_request(addr, "GET", "/api/admin/stats", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "stats");

    // Non-existent nested path returns 404.
    let (status, _) = http_request(addr, "GET", "/admin/stats", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_group_middleware_inheritance() {
    use std::sync::Mutex;
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next};

    #[derive(Clone)]
    struct Log(std::sync::Arc<Mutex<Vec<&'static str>>>);

    struct TagMw(&'static str);
    impl Middleware<Log> for TagMw {
        fn call(
            &self,
            req: flowgate::Request,
            state: std::sync::Arc<Log>,
            next: Next<Log>,
        ) -> BoxFuture {
            let tag = self.0;
            Box::pin(async move {
                state.0.lock().unwrap().push(tag);
                next.run(req, state).await
            })
        }
    }

    let log = Log(std::sync::Arc::new(Mutex::new(Vec::new())));

    async fn handler(State(log): State<Log>) -> &'static str {
        log.0.lock().unwrap().push("handler");
        "ok"
    }

    // App middleware + group middleware should both apply.
    // Group middleware is route-local, app middleware is global.
    // Final order: app_mw → group_mw → handler
    let app = App::with_state(log.clone())
        .layer(TagMw("app"))
        .group(
            Group::new("/api")
                .layer(TagMw("group"))
                .get("/test", handler)
                .unwrap(),
        );
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "GET", "/api/test", None).await;
    assert_eq!(status, StatusCode::OK);

    let entries = log.0.lock().unwrap();
    assert_eq!(*entries, vec!["app", "group", "handler"]);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_group_with_path_params() {
    async fn get_user(Path(id): Path<u64>) -> String {
        format!("user-{id}")
    }

    let app = App::new().group(
        Group::new("/api/v1")
            .get("/users/{id}", get_user)
            .unwrap(),
    );
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/api/v1/users/42", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "user-42");
}

// --- Group path validation ---

#[test]
fn group_route_rejects_invalid_path() {
    async fn handler() -> &'static str { "ok" }

    let result: Result<Group<()>, _> = Group::new("/api").get("users", handler);
    let err = result.err().expect("expected error for invalid path");
    assert!(err.to_string().contains("must start with '/'"));
}

#[test]
fn group_route_accepts_empty_path() {
    async fn handler() -> &'static str { "ok" }

    let result: Result<Group<()>, _> = Group::new("/api").route(http::Method::GET, "", handler);
    assert!(result.is_ok());
}

// --- Built-in Middleware ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_request_id_generated() {
    use flowgate::RequestIdMiddleware;

    async fn handler() -> &'static str {
        "ok"
    }

    let app = App::new()
        .pre(RequestIdMiddleware)
        .get("/test", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, headers, _body) = http_request_with_headers(addr, "GET", "/test", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    // Should have an x-request-id header in the response.
    assert!(headers.contains_key("x-request-id"));
    let id = &headers["x-request-id"];
    assert!(!id.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_request_id_propagated() {
    use flowgate::RequestIdMiddleware;

    async fn handler() -> &'static str {
        "ok"
    }

    let app = App::new()
        .pre(RequestIdMiddleware)
        .get("/test", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, headers, _body) = http_request_with_headers(
        addr,
        "GET",
        "/test",
        None,
        &[("x-request-id", "my-custom-id")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-request-id"], "my-custom-id");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_request_id_extractor() {
    use flowgate::middleware::request_id::RequestId;
    use flowgate::RequestIdMiddleware;

    async fn handler(rid: RequestId) -> String {
        rid.as_str().to_owned()
    }

    let app = App::new()
        .pre(RequestIdMiddleware)
        .get("/test", handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, headers, body) = http_request_with_headers(
        addr,
        "GET",
        "/test",
        None,
        &[("x-request-id", "test-123")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "test-123");
    assert_eq!(headers["x-request-id"], "test-123");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_timeout_middleware() {
    use flowgate::TimeoutMiddleware;
    use std::time::Duration;

    async fn slow_handler() -> &'static str {
        tokio::time::sleep(Duration::from_secs(5)).await;
        "should not reach"
    }

    let app = App::new()
        .layer(TimeoutMiddleware::new(Duration::from_millis(50)))
        .get("/slow", slow_handler)
        .unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/slow", None).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(body, "request timeout");
}

// --- OpenAPI ---

#[cfg(feature = "openapi")]
#[tokio::test(flavor = "current_thread")]
async fn round_trip_openapi_json() {
    use flowgate::OperationMeta;

    async fn health() -> &'static str {
        "ok"
    }

    let app = App::new()
        .meta(flowgate::AppMeta::new("Test API", "1.0.0"))
        .get_with(
            "/health",
            health,
            OperationMeta::new()
                .summary("Health check")
                .tag("ops"),
        )
        .unwrap()
        .with_openapi();
    let (addr, _handle) = serve_app(app).await;

    // /openapi.json should return valid JSON with OpenAPI structure.
    let (status, body) = http_request(addr, "GET", "/openapi.json", None).await;
    assert_eq!(status, StatusCode::OK);
    let spec: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(spec["openapi"], "3.1.0");
    assert_eq!(spec["info"]["title"], "Test API");
    assert_eq!(spec["info"]["version"], "1.0.0");
    // Check that the /health endpoint is documented.
    assert!(spec["paths"]["/health"]["get"].is_object());
    assert_eq!(spec["paths"]["/health"]["get"]["summary"], "Health check");
}

#[cfg(feature = "openapi")]
#[tokio::test(flavor = "current_thread")]
async fn round_trip_openapi_docs_html() {
    async fn handler() -> &'static str {
        "ok"
    }

    let app = App::new()
        .get("/test", handler)
        .unwrap()
        .with_openapi();
    let (addr, _handle) = serve_app(app).await;

    // /docs should return HTML.
    let (status, headers, body) =
        http_request_with_headers(addr, "GET", "/docs", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-type"].contains("text/html"));
    assert!(body.contains("api-reference"));
}

#[cfg(feature = "openapi")]
#[tokio::test(flavor = "current_thread")]
async fn openapi_route_conflict_detected() {
    async fn handler() -> &'static str {
        "ok"
    }

    // User registers /docs — should conflict with OpenAPI docs route.
    let app = App::new()
        .get("/docs", handler)
        .unwrap()
        .with_openapi();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ServerConfig::new().enable_default_tracing(false);
    let result = flowgate::server::serve_with_listener(app, config, listener).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("conflicts with OpenAPI"));
}

// --- Connection limit ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_connection_limit() {
    async fn slow_handler() -> &'static str {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        "done"
    }

    let app = App::new().get("/slow", slow_handler).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = ServerConfig::new()
        .max_connections(Some(1))
        .enable_default_tracing(false);

    let _handle = flowgate::server::serve_with_listener(app, config, listener)
        .await
        .unwrap();

    // First connection should work.
    let (status, _) = http_request(addr, "GET", "/slow", None).await;
    assert_eq!(status, StatusCode::OK);
}

// --- Graceful shutdown ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_graceful_shutdown() {
    async fn handler() -> &'static str {
        "ok"
    }

    let app = App::new().get("/ping", handler).unwrap();
    let (addr, handle) = serve_app(app).await;

    // Make a request to verify the server is running.
    let (status, body) = http_request(addr, "GET", "/ping", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");

    // Shut down and verify it completes.
    handle.shutdown().await.unwrap();
}
