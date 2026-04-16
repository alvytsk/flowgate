use http::request::Parts;
use serde::Deserialize;

use crate::body::Request;
use crate::error::QueryRejection;
use crate::extract::{FromRequest, FromRequestParts};

/// Extractor that deserializes query parameters into `T`.
///
/// ```ignore
/// #[derive(Deserialize)]
/// struct Pagination { page: u32, per_page: u32 }
///
/// async fn list(Query(p): Query<Pagination>) -> String {
///     format!("page {} size {}", p.page, p.per_page)
/// }
/// app.get("/items", list);
/// ```
#[derive(Debug, Clone)]
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: for<'de> Deserialize<'de>,
    S: Send + Sync,
{
    type Rejection = QueryRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or("");
        let value = serde_urlencoded::from_str(query)
            .map_err(|e| QueryRejection::DeserializeError(e.to_string()))?;
        Ok(Query(value))
    }
}

/// Also implement `FromRequest` so `Query<T>` can appear in the last
/// handler argument position (which uses `FromRequest`).
impl<T, S> FromRequest<S> for Query<T>
where
    T: for<'de> Deserialize<'de>,
    S: Send + Sync,
{
    type Rejection = QueryRejection;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, _body) = req.into_parts();
        Self::from_request_parts(&mut parts, state).await
    }
}
