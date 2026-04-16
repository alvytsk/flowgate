use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};

/// Incoming request body — hyper's streaming receive type.
pub type RequestBody = hyper::body::Incoming;

/// Outgoing response body — type-erased, supports both buffered and streaming.
pub type ResponseBody = BoxBody<Bytes, std::convert::Infallible>;

/// Framework-level request type alias.
pub type Request = http::Request<RequestBody>;

/// Framework-level response type alias.
pub type Response = http::Response<ResponseBody>;

/// Wrap bytes into a boxed response body.
pub fn full(body: impl Into<Bytes>) -> ResponseBody {
    Full::new(body.into()).boxed()
}

/// Create an empty response body.
pub fn empty() -> ResponseBody {
    full(Bytes::new())
}
