use http::request::Parts;

use crate::body::Request;
use crate::error::StateRejection;
use crate::extract::{FromRef, FromRequest, FromRequestParts};

/// Extractor that yields application state (or a sub-state via `FromRef`).
#[derive(Debug, Clone, Copy)]
pub struct State<T>(pub T);

impl<S, T> FromRequestParts<S> for State<T>
where
    T: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = StateRejection;

    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(State(T::from_ref(state)))
    }
}

/// Also implement `FromRequest` so `State<T>` can appear in the last
/// handler argument position (which uses `FromRequest`).
impl<S, T> FromRequest<S> for State<T>
where
    T: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = StateRejection;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, _body) = req.into_parts();
        Self::from_request_parts(&mut parts, state).await
    }
}
