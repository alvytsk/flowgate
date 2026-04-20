pub mod json;
pub mod path;
pub mod query;
pub mod request_id;
pub mod state;

use std::future::Future;

use http::request::Parts;

use crate::body::Request;
use crate::response::IntoResponse;

/// Extract a type from request headers/metadata (does not consume the body).
pub trait FromRequestParts<S>: Sized {
    /// Type returned when extraction fails. Converted into a response via
    /// [`IntoResponse`].
    type Rejection: IntoResponse;

    /// Extract `Self` from the request parts. Called by the handler machinery
    /// for every extractor argument except the last.
    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

/// Extract a type from the full request (may consume the body).
pub trait FromRequest<S>: Sized {
    /// Type returned when extraction fails. Converted into a response via
    /// [`IntoResponse`].
    type Rejection: IntoResponse;

    /// Extract `Self` from the full request. Called by the handler machinery
    /// for the last extractor argument (which is allowed to consume the body).
    fn from_request(
        req: Request,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

/// Project a sub-state `T` from the application state `S`.
pub trait FromRef<S> {
    /// Project `Self` out of the full application state.
    fn from_ref(state: &S) -> Self;
}

/// Identity impl: extract `S` itself when `S: Clone`.
impl<S: Clone> FromRef<S> for S {
    fn from_ref(state: &S) -> Self {
        state.clone()
    }
}
