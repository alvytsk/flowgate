pub mod app;
pub mod body;
pub mod config;
pub mod context;
pub mod error;
pub mod extract;
pub mod group;
pub mod handler;
pub mod middleware;
pub mod observer;
pub mod response;
pub mod router;
pub mod server;

#[cfg(feature = "tls")]
pub mod tls;

#[cfg(feature = "openapi")]
pub mod openapi;

#[cfg(not(feature = "openapi"))]
pub mod openapi_stub;

// Re-exports for ergonomic use
pub use app::{App, AppMeta};
pub use body::{Request, Response};
pub use config::ServerConfig;
pub use context::{RequestContext, RouteParams};
pub use error::{BoxError, RouteError};
#[cfg(feature = "tls")]
pub use tls::{TlsConfig, TlsError};
pub use extract::json::Json;
pub use extract::path::Path;
pub use extract::query::Query;
pub use extract::state::State;
pub use extract::FromRef;
pub use group::Group;
pub use handler::Handler;
#[cfg(feature = "recover")]
pub use middleware::recover::RecoverMiddleware;
pub use middleware::request_id::{RequestId, RequestIdMiddleware};
pub use middleware::timeout::TimeoutMiddleware;
pub use middleware::Middleware;
pub use middleware::PreMiddleware;
pub use observer::{MetricsObserver, RequestEvent};
pub use response::IntoResponse;
pub use server::ServerHandle;

// OperationMeta: real type when openapi feature is on, zero-size stub when off.
#[cfg(feature = "openapi")]
pub use openapi::meta::OperationMeta;
#[cfg(not(feature = "openapi"))]
pub use openapi_stub::OperationMeta;
