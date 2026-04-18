use std::convert::Infallible;

use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, StreamBody};

use crate::error::BoxError;

/// Incoming request body — hyper's streaming receive type.
pub type RequestBody = hyper::body::Incoming;

/// Outgoing response body — type-erased, supports both buffered and streaming.
///
/// Uses the unsync variant because streaming response bodies (SSE generators,
/// async user code) are typically `!Sync`, and hyper only needs `Send`.
pub type ResponseBody = UnsyncBoxBody<Bytes, BoxError>;

/// Framework-level request type alias.
pub type Request = http::Request<RequestBody>;

/// Framework-level response type alias.
pub type Response = http::Response<ResponseBody>;

/// Wrap bytes into a boxed response body.
pub fn full(body: impl Into<Bytes>) -> ResponseBody {
    Full::new(body.into())
        .map_err(|err: Infallible| match err {})
        .boxed_unsync()
}

/// Create an empty response body.
pub fn empty() -> ResponseBody {
    full(Bytes::new())
}

/// Wrap a `Stream` of `Bytes` chunks into a boxed response body.
///
/// Each item is framed as a data frame; transport errors are erased into
/// [`BoxError`]. Trailers and raw-frame control are out of scope for v0.2.
pub fn stream<S, E>(s: S) -> ResponseBody
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: Into<BoxError>,
{
    let framed = s.map(|res| res.map(Frame::data).map_err(Into::into));
    StreamBody::new(framed).boxed_unsync()
}
