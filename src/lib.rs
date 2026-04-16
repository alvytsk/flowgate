pub mod app;
pub mod body;
pub mod config;
pub mod context;
pub mod error;
pub mod extract;
pub mod handler;
pub mod middleware;
pub mod response;
pub mod router;
pub mod server;

// Re-exports for ergonomic use
pub use app::App;
pub use body::{Request, Response};
pub use config::ServerConfig;
pub use error::RouteError;
pub use context::{RequestContext, RouteParams};
pub use extract::json::Json;
pub use extract::path::Path;
pub use extract::query::Query;
pub use extract::state::State;
pub use extract::FromRef;
pub use handler::Handler;
pub use middleware::Middleware;
pub use response::IntoResponse;
