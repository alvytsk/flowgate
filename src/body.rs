use bytes::Bytes;
use http_body_util::Full;

/// Incoming request body — hyper's streaming receive type.
pub type RequestBody = hyper::body::Incoming;

/// Outgoing response body — whole body in memory.
pub type ResponseBody = Full<Bytes>;

/// Framework-level request type alias.
pub type Request = http::Request<RequestBody>;

/// Framework-level response type alias.
pub type Response = http::Response<ResponseBody>;
