use bytes::Bytes;
use http::StatusCode;
use http_body_util::Full;

use crate::body::Response;

/// Convert a type into an HTTP response.
pub trait IntoResponse {
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        let mut res = http::Response::new(Full::new(Bytes::from(self)));
        res.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        res
    }
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Response {
        let mut res = http::Response::new(Full::new(Bytes::from_static(self.as_bytes())));
        res.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        res
    }
}

impl IntoResponse for StatusCode {
    fn into_response(self) -> Response {
        let mut res = http::Response::new(Full::new(Bytes::new()));
        *res.status_mut() = self;
        res
    }
}

impl IntoResponse for (StatusCode, String) {
    fn into_response(self) -> Response {
        let mut res = self.1.into_response();
        *res.status_mut() = self.0;
        res
    }
}

impl IntoResponse for (StatusCode, &'static str) {
    fn into_response(self) -> Response {
        let mut res = self.1.into_response();
        *res.status_mut() = self.0;
        res
    }
}

/// Blanket impl: Result<T, E> where both T and E implement IntoResponse.
impl<T, E> IntoResponse for Result<T, E>
where
    T: IntoResponse,
    E: IntoResponse,
{
    fn into_response(self) -> Response {
        match self {
            Ok(v) => v.into_response(),
            Err(e) => e.into_response(),
        }
    }
}
