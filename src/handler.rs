use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::body::{Request, Response};
use crate::response::IntoResponse;

/// A boxed future used for type-erased handler dispatch.
pub type BoxFuture = Pin<Box<dyn Future<Output = Response> + Send + 'static>>;

/// User-facing handler trait — generic over extractor tuples.
pub trait Handler<T, S>: Clone + Send + 'static {
    fn call(self, req: Request, state: Arc<S>) -> impl Future<Output = Response> + Send;
}

/// Object-safe trait stored in the router. Takes `Arc<S>` so the returned
/// `BoxFuture` is `'static` without lifetime issues.
pub(crate) trait Endpoint<S>: Send + Sync + 'static {
    fn call(&self, req: Request, state: Arc<S>) -> BoxFuture;
}

/// Bridge struct that converts `Handler<T, S>` into `Arc<dyn Endpoint<S>>`.
pub(crate) struct HandlerEndpoint<H, T> {
    handler: H,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<H: Clone, T> Clone for HandlerEndpoint<H, T> {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<H, T, S> Endpoint<S> for HandlerEndpoint<H, T>
where
    H: Handler<T, S> + Send + Sync + 'static,
    T: Send + 'static,
    S: Send + Sync + 'static,
{
    fn call(&self, req: Request, state: Arc<S>) -> BoxFuture {
        let handler = self.handler.clone();
        Box::pin(handler.call(req, state))
    }
}

/// Convert a handler function into an `Arc<dyn Endpoint<S>>`.
pub(crate) fn into_endpoint<H, T, S>(handler: H) -> Arc<dyn Endpoint<S>>
where
    H: Handler<T, S> + Send + Sync + 'static,
    T: Send + 'static,
    S: Send + Sync + 'static,
{
    Arc::new(HandlerEndpoint {
        handler,
        _marker: std::marker::PhantomData,
    })
}

// --- Zero-arg handler adapter ---

impl<F, Fut, Res, S> Handler<(), S> for F
where
    F: FnOnce() -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Res> + Send,
    Res: IntoResponse,
    S: Send + Sync + 'static,
{
    async fn call(self, _req: Request, _state: Arc<S>) -> Response {
        self().await.into_response()
    }
}

// --- Macro-generated adapters for 1..N extractors ---
//
// The last extractor position uses `FromRequest` (may consume the body).
// All preceding positions use `FromRequestParts` (header-only).
// A blanket impl of `FromRequest` for `FromRequestParts` types (in extract/mod.rs)
// allows parts-only extractors in the last position too.

macro_rules! impl_handler {
    // Single extractor — it gets the full request via FromRequest.
    ( $last:ident ) => {
        #[allow(non_snake_case)]
        impl<F, Fut, Res, S, $last> Handler<($last,), S> for F
        where
            F: FnOnce($last) -> Fut + Clone + Send + 'static,
            Fut: Future<Output = Res> + Send,
            Res: IntoResponse,
            S: Send + Sync + 'static,
            $last: crate::extract::FromRequest<S> + Send,
        {
            async fn call(self, req: Request, state: Arc<S>) -> Response {
                let $last = match <$last as crate::extract::FromRequest<S>>::from_request(req, &*state).await {
                    Ok(v) => v,
                    Err(rej) => return rej.into_response(),
                };
                self($last).await.into_response()
            }
        }
    };
    // N extractors — first N-1 are FromRequestParts, last is FromRequest.
    ( $($ty:ident),+ ; $last:ident ) => {
        #[allow(non_snake_case)]
        impl<F, Fut, Res, S, $($ty,)+ $last> Handler<($($ty,)+ $last,), S> for F
        where
            F: FnOnce($($ty,)+ $last) -> Fut + Clone + Send + 'static,
            Fut: Future<Output = Res> + Send,
            Res: IntoResponse,
            S: Send + Sync + 'static,
            $( $ty: crate::extract::FromRequestParts<S> + Send, )+
            $last: crate::extract::FromRequest<S> + Send,
        {
            async fn call(self, req: Request, state: Arc<S>) -> Response {
                let (mut parts, body) = req.into_parts();
                $(
                    let $ty = match <$ty as crate::extract::FromRequestParts<S>>::from_request_parts(&mut parts, &*state).await {
                        Ok(v) => v,
                        Err(rej) => return rej.into_response(),
                    };
                )+
                let req = http::Request::from_parts(parts, body);
                let $last = match <$last as crate::extract::FromRequest<S>>::from_request(req, &*state).await {
                    Ok(v) => v,
                    Err(rej) => return rej.into_response(),
                };
                self($($ty,)+ $last).await.into_response()
            }
        }
    };
}

impl_handler!(T1);
impl_handler!(T1; T2);
impl_handler!(T1, T2; T3);
impl_handler!(T1, T2, T3; T4);
impl_handler!(T1, T2, T3, T4; T5);
impl_handler!(T1, T2, T3, T4, T5; T6);
impl_handler!(T1, T2, T3, T4, T5, T6; T7);
impl_handler!(T1, T2, T3, T4, T5, T6, T7; T8);
