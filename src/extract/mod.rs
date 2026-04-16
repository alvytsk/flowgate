pub mod json;
pub mod state;

use std::future::Future;

use http::request::Parts;

use crate::body::Request;
use crate::response::IntoResponse;

/// Extract a type from request headers/metadata (does not consume the body).
pub trait FromRequestParts<S>: Sized {
    type Rejection: IntoResponse;

    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

/// Extract a type from the full request (may consume the body).
pub trait FromRequest<S>: Sized {
    type Rejection: IntoResponse;

    fn from_request(
        req: Request,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

/// Project a sub-state `T` from the application state `S`.
pub trait FromRef<S> {
    fn from_ref(state: &S) -> Self;
}

/// Identity impl: extract `S` itself when `S: Clone`.
impl<S: Clone> FromRef<S> for S {
    fn from_ref(state: &S) -> Self {
        state.clone()
    }
}
