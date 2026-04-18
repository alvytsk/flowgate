use http::StatusCode;

use crate::body::Request;
use crate::extract::{FromRequest, FromRequestParts};
use crate::middleware::request_id::RequestId;
use crate::response::IntoResponse;

/// Extractor for the request ID set by `RequestIdMiddleware`.
///
/// Rejects with 500 Internal Server Error if the middleware is not installed,
/// since that is a framework wiring error, not a client error.
///
/// Implements both `FromRequestParts` and `FromRequest` so it works in any
/// handler argument position (same pattern as `State<T>`, `Path<T>`).
impl<S: Send + Sync> FromRequestParts<S> for RequestId {
    type Rejection = RequestIdRejection;

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<RequestId>()
            .cloned()
            .ok_or(RequestIdRejection)
    }
}

impl<S: Send + Sync> FromRequest<S> for RequestId {
    type Rejection = RequestIdRejection;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, _body) = req.into_parts();
        Self::from_request_parts(&mut parts, state).await
    }
}

/// Rejection returned when `RequestIdMiddleware` is not installed.
#[derive(Debug)]
pub struct RequestIdRejection;

impl std::fmt::Display for RequestIdRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RequestIdMiddleware not installed")
    }
}

impl std::error::Error for RequestIdRejection {}

impl IntoResponse for RequestIdRejection {
    fn into_response(self) -> crate::body::Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error: request ID middleware not installed",
        )
            .into_response()
    }
}
