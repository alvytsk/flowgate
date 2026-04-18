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

    let State(name): State<String> = State::from_request_parts(&mut parts, &state).await.unwrap();
    assert_eq!(name, "test");

    let State(count): State<u32> = State::from_request_parts(&mut parts, &state).await.unwrap();
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
        body_read_timeout: Some(std::time::Duration::from_secs(5)),
    };
    let cloned = ctx.clone();
    assert_eq!(cloned.body_limit, 1024);
    assert_eq!(
        cloned.body_read_timeout,
        Some(std::time::Duration::from_secs(5))
    );
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
    assert_eq!(
        config.body_read_timeout,
        Some(std::time::Duration::from_secs(30))
    );
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
        .body_read_timeout(None)
        .keep_alive(false)
        .header_read_timeout(None)
        .max_headers(None)
        .enable_default_tracing(false);

    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3000);
    assert_eq!(config.bind_addr(), "127.0.0.1:3000");
    assert_eq!(config.json_body_limit, 1024);
    assert!(config.body_read_timeout.is_none());
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

    let r = JsonRejection::BodyReadTimeout;
    assert_eq!(r.to_string(), "body read timeout");
}

#[test]
fn json_rejection_into_response() {
    use flowgate::error::JsonRejection;

    let res = JsonRejection::PayloadTooLarge.into_response();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(res.headers().get(http::header::CONNECTION).is_none());

    let res = JsonRejection::BodyReadError("err".into()).into_response();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(res.headers().get(http::header::CONNECTION).is_none());

    let res = JsonRejection::BodyReadTimeout.into_response();
    assert_eq!(res.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(
        res.headers()
            .get(http::header::CONNECTION)
            .and_then(|v| v.to_str().ok()),
        Some("close"),
    );
}

// --- Path extraction ---

#[tokio::test(flavor = "current_thread")]
async fn path_single_string() {
    use flowgate::context::{RequestContext, RouteParams};
    use flowgate::extract::FromRequestParts;

    let req = http::Request::builder()
        .uri("/users/alice")
        .body(())
        .unwrap();
    let (mut parts, _) = req.into_parts();
    parts.extensions.insert(RequestContext {
        route_params: RouteParams(vec![("name".into(), "alice".into())]),
        body_limit: 262_144,
        body_read_timeout: None,
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
        body_read_timeout: None,
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
        body_read_timeout: None,
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
        body_read_timeout: None,
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
        body_read_timeout: None,
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
) -> (
    StatusCode,
    std::collections::HashMap<String, String>,
    String,
) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = hyper_util::rt::TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
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

    let app = App::with_state(Counter { value: 99 })
        .get("/count", get_count)
        .unwrap();
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

    let (status, body) = http_request(addr, "PUT", "/users/7", Some(r#"{"name":"alice"}"#)).await;
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
    assert_eq!(
        headers.get("content-type").unwrap(),
        "text/plain; charset=utf-8"
    );
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
    async fn handler() -> &'static str {
        "ok"
    }

    let app = App::new().get("/exists", handler).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, _body) = http_request(addr, "HEAD", "/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_405_allow_header_includes_head() {
    async fn handler() -> &'static str {
        "ok"
    }

    let app = App::new().get("/item", handler).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, headers, _body) =
        http_request_with_headers(addr, "DELETE", "/item", None, &[]).await;
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
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next};
    use std::sync::Mutex;

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

    let (status, body) = http_request(addr, "POST", "/items/5", Some(r#"{"value":"hello"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["result"], "pfx-5-hello");
}

// --- Pre-routing middleware ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_pre_middleware_ordering() {
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next, PreMiddleware, PreNext};
    use std::sync::Mutex;

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

    let app = App::new().pre(BlockPre).get("/secret", handler).unwrap();
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/secret", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, "pre-blocked");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_layer_after_route_still_applies() {
    // Verifies that builder order doesn't matter: .layer() after .get()
    // still applies the middleware to the route (finalization merges them).
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next};
    use std::sync::Mutex;

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
        .group(Group::new("/api").get("/users", list_users).unwrap());
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

    let app = App::new().group(
        Group::new("/api")
            .get("/users", users)
            .unwrap()
            .group(Group::new("/admin").get("/stats", stats).unwrap()),
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
    use flowgate::handler::BoxFuture;
    use flowgate::middleware::{Middleware, Next};
    use std::sync::Mutex;

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
    let app = App::with_state(log.clone()).layer(TagMw("app")).group(
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

    let app = App::new().group(Group::new("/api/v1").get("/users/{id}", get_user).unwrap());
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/api/v1/users/42", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "user-42");
}

// --- Group path validation ---

#[test]
fn group_route_rejects_invalid_path() {
    async fn handler() -> &'static str {
        "ok"
    }

    let result: Result<Group<()>, _> = Group::new("/api").get("users", handler);
    let err = result.err().expect("expected error for invalid path");
    assert!(err.to_string().contains("must start with '/'"));
}

#[test]
fn group_route_accepts_empty_path() {
    async fn handler() -> &'static str {
        "ok"
    }

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

    let (status, headers, body) =
        http_request_with_headers(addr, "GET", "/test", None, &[("x-request-id", "test-123")])
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
            OperationMeta::new().summary("Health check").tag("ops"),
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

    let app = App::new().get("/test", handler).unwrap().with_openapi();
    let (addr, _handle) = serve_app(app).await;

    // /docs should return HTML.
    let (status, headers, body) = http_request_with_headers(addr, "GET", "/docs", None, &[]).await;
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
    let app = App::new().get("/docs", handler).unwrap().with_openapi();

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

// --- Metrics observer ---

#[derive(Clone, Debug)]
struct CapturedEvent {
    method: String,
    route_pattern: Option<String>,
    status: u16,
    duration_is_positive: bool,
}

#[derive(Default, Clone)]
struct CapturingObserver {
    events: std::sync::Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
}

impl flowgate::observer::MetricsObserver for CapturingObserver {
    fn on_request(&self, event: &flowgate::observer::RequestEvent<'_>) {
        self.events.lock().unwrap().push(CapturedEvent {
            method: event.method.to_string(),
            route_pattern: event.route_pattern.map(str::to_owned),
            status: event.status.as_u16(),
            duration_is_positive: event.duration > std::time::Duration::ZERO,
        });
    }
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_observer_captures_matched_route_pattern() {
    async fn get_user(Path(id): Path<u64>) -> String {
        format!("user {id}")
    }

    let observer = CapturingObserver::default();
    let app = App::new()
        .get("/users/{id}", get_user)
        .unwrap()
        .observe(observer.clone());
    let (addr, _handle) = serve_app(app).await;

    let (status, body) = http_request(addr, "GET", "/users/42", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "user 42");

    let events = observer.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    // Raw path is `/users/42`; observer must see the *pattern* for
    // bounded-cardinality metrics, not the per-request path.
    assert_eq!(events[0].route_pattern.as_deref(), Some("/users/{id}"));
    assert_eq!(events[0].method, "GET");
    assert_eq!(events[0].status, 200);
    assert!(events[0].duration_is_positive);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_observer_captures_404() {
    async fn noop() -> &'static str {
        "unused"
    }

    let observer = CapturingObserver::default();
    let app = App::new()
        .get("/exists", noop)
        .unwrap()
        .observe(observer.clone());
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "GET", "/missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let events = observer.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].route_pattern, None);
    assert_eq!(events[0].status, 404);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_observer_captures_405() {
    async fn get_only() -> &'static str {
        "ok"
    }

    let observer = CapturingObserver::default();
    let app = App::new()
        .get("/only-get", get_only)
        .unwrap()
        .observe(observer.clone());
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "POST", "/only-get", Some("{}")).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    let events = observer.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].route_pattern, None);
    assert_eq!(events[0].status, 405);
    assert_eq!(events[0].method, "POST");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_multiple_observers_all_fire() {
    async fn ping() -> &'static str {
        "pong"
    }

    let a = CapturingObserver::default();
    let b = CapturingObserver::default();
    let app = App::new()
        .get("/ping", ping)
        .unwrap()
        .observe(a.clone())
        .observe(b.clone());
    let (addr, _handle) = serve_app(app).await;

    let (status, _) = http_request(addr, "GET", "/ping", None).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(a.events.lock().unwrap().len(), 1);
    assert_eq!(b.events.lock().unwrap().len(), 1);
}

// --- Body read timeout ---

#[tokio::test(flavor = "current_thread")]
async fn round_trip_body_read_timeout_stall_returns_408() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Payload {
        x: i32,
    }

    async fn echo(Json(_): Json<Payload>) -> &'static str {
        "ok"
    }

    let app = App::new().post("/echo", echo).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = ServerConfig::new()
        .body_read_timeout(Some(std::time::Duration::from_millis(150)))
        .enable_default_tracing(false);

    let _handle = flowgate::server::serve_with_listener(app, config, listener)
        .await
        .unwrap();

    // Raw HTTP/1.1: announce Content-Length: 100, send 6 bytes, then stall.
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = "POST /echo HTTP/1.1\r\n\
                   Host: localhost\r\n\
                   Content-Type: application/json\r\n\
                   Content-Length: 100\r\n\
                   \r\n\
                   {\"x\":1";
    stream.write_all(request.as_bytes()).await.unwrap();

    // Server should respond inside the 150 ms timeout (with headroom for CI).
    let mut buf = Vec::with_capacity(1024);
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        stream.read_to_end(&mut buf),
    )
    .await;
    assert!(
        read.is_ok(),
        "server did not respond within 2s (no timeout fired)"
    );
    read.unwrap().unwrap();

    let response = String::from_utf8_lossy(&buf);
    assert!(
        response.starts_with("HTTP/1.1 408"),
        "expected 408 status line, got: {response:?}"
    );
    assert!(
        response.to_ascii_lowercase().contains("connection: close"),
        "expected Connection: close header, got: {response:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_body_read_timeout_fast_body_succeeds() {
    #[derive(Deserialize)]
    struct Payload {
        x: i32,
    }
    #[derive(Serialize)]
    struct Out {
        doubled: i32,
    }

    async fn double(Json(input): Json<Payload>) -> Json<Out> {
        Json(Out {
            doubled: input.x * 2,
        })
    }

    let app = App::new().post("/double", double).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Tight timeout, but the client sends the full body immediately — should succeed.
    let config = ServerConfig::new()
        .body_read_timeout(Some(std::time::Duration::from_millis(200)))
        .enable_default_tracing(false);

    let _handle = flowgate::server::serve_with_listener(app, config, listener)
        .await
        .unwrap();

    let (status, body) = http_request(addr, "POST", "/double", Some(r#"{"x":21}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["doubled"], 42);
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

// --- TLS tests ---

#[cfg(feature = "tls")]
mod tls_tests {
    use super::*;
    use flowgate::TlsConfig;
    use http_body_util::Empty;
    use hyper_util::rt::TokioIo;
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName};
    use rustls::{ClientConfig as RustlsClientConfig, RootCertStore};
    use std::io::Write;
    use std::sync::Arc;
    use tokio_rustls::TlsConnector;

    struct GeneratedCert {
        cert_der: CertificateDer<'static>,
        tls_config: TlsConfig,
    }

    fn gen_self_signed_tls() -> GeneratedCert {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let cert_der = cert.cert.der().clone();
        let key_der =
            PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
        let rustls_cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der.clone()], key_der.into())
            .unwrap();
        GeneratedCert {
            cert_der,
            tls_config: TlsConfig::from_rustls(Arc::new(rustls_cfg)),
        }
    }

    fn build_client(trusted: CertificateDer<'static>) -> TlsConnector {
        let mut roots = RootCertStore::empty();
        roots.add(trusted).unwrap();
        let client_cfg = RustlsClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        TlsConnector::from(Arc::new(client_cfg))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_round_trip_https() {
        async fn hello() -> &'static str {
            "secure hello"
        }

        let generated = gen_self_signed_tls();
        let app = App::new().get("/", hello).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = ServerConfig::new()
            .enable_default_tracing(false)
            .tls(generated.tls_config);
        let _handle = flowgate::server::serve_with_listener(app, config, listener)
            .await
            .unwrap();

        let connector = build_client(generated.cert_der);
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let domain = ServerName::try_from("localhost").unwrap();
        let tls_stream = connector.connect(domain, tcp).await.unwrap();

        let io = TokioIo::new(tls_stream);
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
                .await
                .unwrap();
        tokio::spawn(conn);

        let req = http::Request::builder()
            .method("GET")
            .uri("/")
            .header("host", "localhost")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let res = sender.send_request(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body_bytes[..], b"secure hello");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_from_pem_files_round_trip() {
        async fn hello() -> &'static str {
            "pem hello"
        }

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let cert_pem = cert.cert.pem();
        let key_pem = cert.key_pair.serialize_pem();

        let mut cert_file = tempfile::NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let mut key_file = tempfile::NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();

        let tls = TlsConfig::from_pem_files(cert_file.path(), key_file.path()).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = ServerConfig::new().enable_default_tracing(false).tls(tls);
        let app = App::new().get("/", hello).unwrap();
        let _handle = flowgate::server::serve_with_listener(app, config, listener)
            .await
            .unwrap();

        let connector = build_client(cert.cert.der().clone());
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let domain = ServerName::try_from("localhost").unwrap();
        let tls_stream = connector.connect(domain, tcp).await.unwrap();

        let io = TokioIo::new(tls_stream);
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
                .await
                .unwrap();
        tokio::spawn(conn);

        let req = http::Request::builder()
            .method("GET")
            .uri("/")
            .header("host", "localhost")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let res = sender.send_request(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}

// --- SSE tests ---

mod sse_tests {
    use super::*;
    use flowgate::sse::{Event, Sse};
    use futures_core::Stream;
    use futures_util::stream;
    use http_body_util::Empty;
    use hyper_util::rt::TokioIo;
    use std::time::Duration;

    async fn three_events() -> Sse<impl Stream<Item = Event>> {
        let events = stream::iter([
            Event::default().data("one"),
            Event::default().data("two"),
            Event::default().data("three"),
        ]);
        Sse::new(events)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sse_stream_emits_events() {
        let app = App::new().get("/events", three_events).unwrap();
        let (addr, _handle) = serve_app(app).await;

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let io = TokioIo::new(stream);
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
                .await
                .unwrap();
        tokio::spawn(conn);

        let req = http::Request::builder()
            .method("GET")
            .uri("/events")
            .header("host", "localhost")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let res = sender.send_request(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        assert_eq!(
            res.headers().get(http::header::CACHE_CONTROL).unwrap(),
            "no-cache"
        );
        assert_eq!(res.headers().get("x-accel-buffering").unwrap(), "no");

        let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&body_bytes).unwrap();
        assert_eq!(body, "data: one\n\ndata: two\n\ndata: three\n\n");
    }

    async fn pending_with_heartbeat() -> Sse<impl Stream<Item = Event>> {
        // Stream that never yields an event — only heartbeat frames drive the body.
        let never: stream::Pending<Event> = stream::pending();
        Sse::new(never).keep_alive(Duration::from_millis(30))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sse_heartbeat_frames_are_emitted() {
        let app = App::new().get("/events", pending_with_heartbeat).unwrap();
        let (addr, _handle) = serve_app(app).await;

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let io = TokioIo::new(stream);
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io)
                .await
                .unwrap();
        tokio::spawn(conn);

        let req = http::Request::builder()
            .method("GET")
            .uri("/events")
            .header("host", "localhost")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let res = sender.send_request(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::OK);

        let mut body = res.into_body();
        let read = tokio::time::timeout(Duration::from_secs(2), async {
            let frame = body.frame().await.unwrap().unwrap();
            frame.into_data().ok().unwrap()
        })
        .await
        .expect("heartbeat did not arrive within 2s");

        assert_eq!(&read[..], b":\n\n");
    }
}

// --- WebSocket tests ---

#[cfg(feature = "ws")]
mod ws_tests {
    use super::*;
    use flowgate::body::Response;
    use flowgate::ws::{Message, WebSocketUpgrade};
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio_tungstenite::client_async;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    async fn echo(upgrade: WebSocketUpgrade) -> Response {
        upgrade.on_upgrade(|mut socket| async move {
            while let Some(Ok(msg)) = socket.recv().await {
                if msg.is_close() {
                    break;
                }
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ws_echo_round_trip() {
        let app = App::new().get("/ws", echo).unwrap();
        let (addr, _handle) = serve_app(app).await;

        let url = format!("ws://{addr}/ws");
        let stream = TcpStream::connect(addr).await.unwrap();
        let request = url.into_client_request().unwrap();
        let (mut ws_stream, _response) = client_async(request, stream).await.unwrap();

        ws_stream
            .send(Message::Text("hello".into()))
            .await
            .unwrap();

        let echoed = ws_stream.next().await.unwrap().unwrap();
        match echoed {
            Message::Text(text) => assert_eq!(text.as_str(), "hello"),
            other => panic!("expected text message, got {other:?}"),
        }
    }

    /// Regression test for naive string-equality header checking.
    /// A spec-compliant `Connection: keep-alive, Upgrade` must succeed.
    #[tokio::test(flavor = "current_thread")]
    async fn ws_accepts_compound_connection_header() {
        let app = App::new().get("/ws", echo).unwrap();
        let (addr, _handle) = serve_app(app).await;

        let mut stream = TcpStream::connect(addr).await.unwrap();

        // Raw HTTP/1.1 upgrade request with a compound Connection header.
        let req = b"GET /ws HTTP/1.1\r\n\
            Host: localhost\r\n\
            Connection: keep-alive, Upgrade\r\n\
            Upgrade: websocket\r\n\
            Sec-WebSocket-Version: 13\r\n\
            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
            \r\n";
        stream.write_all(req).await.unwrap();

        // Read just the status line and first header batch.
        let mut buf = vec![0u8; 1024];
        let n = stream.read(&mut buf).await.unwrap();
        let resp = std::str::from_utf8(&buf[..n]).unwrap();

        assert!(
            resp.starts_with("HTTP/1.1 101 Switching Protocols"),
            "expected 101 response, got:\n{resp}"
        );
        // Expected Sec-WebSocket-Accept for the canonical example key.
        assert!(
            resp.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
            "expected canonical Sec-WebSocket-Accept header; got:\n{resp}"
        );
    }
}
