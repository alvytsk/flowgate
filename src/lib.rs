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
pub mod sse;

#[cfg(feature = "tls")]
pub mod tls;

#[cfg(feature = "ws")]
pub mod ws;

pub mod openapi;

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
pub use sse::{Event, Sse};
#[cfg(feature = "ws")]
pub use ws::{Message, WebSocket, WebSocketUpgrade, WsError};

// OperationMeta: real type when openapi feature is on, zero-size stub when off.
pub use openapi::OperationMeta;

// Ergonomic re-exports from common upstream crates so users don't need parallel
// `http` / `bytes` imports in their `Cargo.toml`.
pub use bytes::Bytes;
pub use http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
