use crate::body::Response;
use crate::response::IntoResponse;
use http::StatusCode;

/// Rejection returned when JSON extraction fails.
#[derive(Debug)]
pub enum JsonRejection {
    /// Request body exceeded the configured size limit.
    PayloadTooLarge,
    /// Failed to read the request body.
    BodyReadError(String),
    /// Failed to deserialize the JSON body.
    InvalidJson(serde_json::Error),
}

impl std::fmt::Display for JsonRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadTooLarge => write!(f, "payload too large"),
            Self::BodyReadError(msg) => write!(f, "failed to read body: {msg}"),
            Self::InvalidJson(err) => write!(f, "invalid JSON: {err}"),
        }
    }
}

impl std::error::Error for JsonRejection {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidJson(err) => Some(err),
            _ => None,
        }
    }
}

impl IntoResponse for JsonRejection {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::BodyReadError(_) => StatusCode::BAD_REQUEST,
            Self::InvalidJson(_) => StatusCode::UNPROCESSABLE_ENTITY,
        };
        (status, self.to_string()).into_response()
    }
}

/// Rejection returned when path parameter extraction fails.
#[derive(Debug)]
pub enum PathRejection {
    /// No route parameters available (handler called outside router context).
    MissingRouteParams,
    /// Failed to deserialize path parameters into the target type.
    DeserializeError(String),
}

impl std::fmt::Display for PathRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRouteParams => write!(f, "no route parameters found"),
            Self::DeserializeError(msg) => write!(f, "invalid path parameters: {msg}"),
        }
    }
}

impl std::error::Error for PathRejection {}

impl IntoResponse for PathRejection {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::MissingRouteParams => StatusCode::INTERNAL_SERVER_ERROR,
            Self::DeserializeError(_) => StatusCode::BAD_REQUEST,
        };
        (status, self.to_string()).into_response()
    }
}

/// Rejection returned when query parameter extraction fails.
#[derive(Debug)]
pub enum QueryRejection {
    /// Failed to deserialize query parameters into the target type.
    DeserializeError(String),
}

impl std::fmt::Display for QueryRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeserializeError(msg) => write!(f, "invalid query parameters: {msg}"),
        }
    }
}

impl std::error::Error for QueryRejection {}

impl IntoResponse for QueryRejection {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, self.to_string()).into_response()
    }
}

/// Error returned when route registration fails (e.g., conflicting path patterns).
#[derive(Debug)]
pub struct RouteError(pub(crate) String);

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "route registration failed: {}", self.0)
    }
}

impl std::error::Error for RouteError {}

/// Rejection returned when state extraction fails (infallible in practice).
#[derive(Debug)]
pub enum StateRejection {
    /// State type not available (should not happen if App is configured correctly).
    MissingState,
}

impl std::fmt::Display for StateRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingState => write!(f, "missing state"),
        }
    }
}

impl std::error::Error for StateRejection {}

impl IntoResponse for StateRejection {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}
