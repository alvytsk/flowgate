use std::sync::Arc;

use flowgate::extract::json::Json;
use flowgate::extract::state::State;
use flowgate::extract::FromRef;
use flowgate::middleware::TracingMiddleware;
use flowgate::{App, ServerConfig};
use serde::{Deserialize, Serialize};

// --- Application state ---

#[derive(Clone)]
struct AppState {
    db: Arc<Db>,
    app_name: String,
}

struct Db {
    healthy: bool,
}

// Sub-state projection: extract Arc<Db> from AppState cheaply.
impl FromRef<AppState> for Arc<Db> {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

// Sub-state projection: extract the app name.
impl FromRef<AppState> for String {
    fn from_ref(state: &AppState) -> Self {
        state.app_name.clone()
    }
}

// --- Request/Response types ---

#[derive(Deserialize)]
struct EchoRequest {
    msg: String,
}

#[derive(Serialize)]
struct EchoResponse {
    echo: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    app: String,
    db_healthy: bool,
}

// --- Handlers ---

async fn health(State(db): State<Arc<Db>>, State(app_name): State<String>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        app: app_name,
        db_healthy: db.healthy,
    })
}

async fn echo(Json(body): Json<EchoRequest>) -> Json<EchoResponse> {
    Json(EchoResponse {
        echo: format!("you said: {}", body.msg),
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        db: Arc::new(Db { healthy: true }),
        app_name: "flowgate-hello".to_owned(),
    };

    let app = App::with_state(state)
        .layer(TracingMiddleware)
        .get("/health", health)?
        .post("/echo", echo)?;

    let config = ServerConfig::from_env();

    let _handle = flowgate::server::serve(app, config).await?;

    // Block forever (Ctrl+C terminates the process).
    std::future::pending::<()>().await;
    Ok(())
}
