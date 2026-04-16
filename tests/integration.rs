use bytes::Bytes;
use http::StatusCode;
use http_body_util::{BodyExt, Full};
use serde::{Deserialize, Serialize};

use flowgate::context::{RequestContext, RouteParams};
use flowgate::extract::json::Json;
use flowgate::extract::state::State;
use flowgate::extract::FromRef;
use flowgate::response::IntoResponse;
use flowgate::{App, ServerConfig};

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
    assert_eq!(config.addr, "0.0.0.0:8080");
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

    assert_eq!(config.addr, "127.0.0.1:3000");
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

// --- Full server round-trip tests ---

/// Helper: spin up a Flowgate server on a random port, return the address.
async fn serve_app<S: Send + Sync + 'static>(app: App<S>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = ServerConfig::new()
        .addr(addr.to_string())
        .enable_default_tracing(false);

    // We can't reuse the listener with `serve`, so re-bind inside `serve`.
    // Drop our listener and let `serve` bind to the same address.
    drop(listener);

    tokio::spawn(async move {
        flowgate::server::serve(app, config).await.unwrap();
    });

    // Give the server a moment to bind.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

/// Make an HTTP/1.1 request and return the response.
async fn http_request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (StatusCode, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = hyper_util::rt::TokioIo::new(stream);

    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.unwrap();
    tokio::spawn(conn);

    let req_body = match body {
        Some(b) => Full::new(Bytes::from(b.to_owned())),
        None => Full::new(Bytes::new()),
    };

    let req = http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("content-type", "application/json")
        .body(req_body)
        .unwrap();

    let res = sender.send_request(req).await.unwrap();
    let status = res.status();
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

    (status, body_str)
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

    let app = App::new().get("/ping", ping);
    let addr = serve_app(app).await;

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

    let app = App::new().post("/double", double);
    let addr = serve_app(app).await;

    let (status, body) = http_request(addr, "POST", "/double", Some(r#"{"x":21}"#)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["doubled"], 42);
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip_404() {
    let app = App::new();
    let addr = serve_app(app).await;

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

    let app = App::with_state(Counter { value: 99 }).get("/count", get_count);
    let addr = serve_app(app).await;

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

    let app = App::new().post("/big", accept_big);

    // Use a tiny body limit
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let config = ServerConfig::new()
        .addr(addr.to_string())
        .json_body_limit(32) // 32 bytes
        .enable_default_tracing(false);

    tokio::spawn(async move {
        flowgate::server::serve(app, config).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send a body larger than 32 bytes
    let big_payload = r#"{"data":"this string is definitely longer than thirty-two bytes"}"#;
    let (status, _) = http_request(addr, "POST", "/big", Some(big_payload)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}
