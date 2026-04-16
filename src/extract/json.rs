use bytes::Bytes;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::body::{full, Request, Response};
use crate::config::DEFAULT_JSON_BODY_LIMIT;
use crate::context::RequestContext;
use crate::error::JsonRejection;
use crate::extract::FromRequest;
use crate::response::IntoResponse;

/// JSON extractor and responder.
///
/// As an extractor, deserializes the request body as JSON.
/// As a responder, serializes `T` into a JSON response.
#[derive(Debug, Clone)]
pub struct Json<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for Json<T> {
    type Rejection = JsonRejection;

    async fn from_request(req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let (parts, body) = req.into_parts();

        let limit = parts
            .extensions
            .get::<RequestContext>()
            .map(|ctx| ctx.body_limit)
            .unwrap_or(DEFAULT_JSON_BODY_LIMIT);

        let limited = Limited::new(body, limit);
        let collected = limited.collect().await.map_err(|e| {
            if e.downcast_ref::<LengthLimitError>().is_some() {
                JsonRejection::PayloadTooLarge
            } else {
                JsonRejection::BodyReadError(e.to_string())
            }
        })?;
        let bytes = collected.to_bytes();
        let value = serde_json::from_slice(&bytes).map_err(JsonRejection::InvalidJson)?;
        Ok(Json(value))
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        match serde_json::to_vec(&self.0) {
            Ok(body) => {
                let mut res = http::Response::new(full(Bytes::from(body)));
                res.headers_mut().insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                );
                res
            }
            Err(_) => {
                let mut res = http::Response::new(full(Bytes::from(
                    r#"{"error":"failed to serialize response"}"#,
                )));
                *res.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
                res.headers_mut().insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                );
                res
            }
        }
    }
}
