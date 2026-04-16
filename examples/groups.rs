use std::sync::Arc;
use std::time::Duration;

use flowgate::extract::json::Json;
use flowgate::extract::path::Path;
use flowgate::extract::state::State;
use flowgate::extract::FromRef;
use flowgate::middleware::TracingMiddleware;
use flowgate::{App, AppMeta, Group, RequestIdMiddleware, ServerConfig, TimeoutMiddleware};
use serde::{Deserialize, Serialize};

// --- Application state ---

#[derive(Clone)]
struct AppState {
    db: Arc<Db>,
}

struct Db {
    healthy: bool,
}

impl FromRef<AppState> for Arc<Db> {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

// --- Request/Response types ---

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
}

#[derive(Deserialize)]
struct CreateUser {
    name: String,
}

#[derive(Serialize)]
struct AdminStats {
    users: u64,
    uptime_secs: u64,
}

// --- Handlers ---

async fn health(State(db): State<Arc<Db>>) -> Json<HealthResponse> {
    let status = if db.healthy { "ok" } else { "degraded" };
    Json(HealthResponse { status })
}

async fn get_user(Path(id): Path<u64>) -> Json<User> {
    Json(User {
        id,
        name: format!("user-{id}"),
    })
}

async fn create_user(Json(body): Json<CreateUser>) -> Json<User> {
    Json(User {
        id: 1,
        name: body.name,
    })
}

async fn admin_stats() -> Json<AdminStats> {
    Json(AdminStats {
        users: 42,
        uptime_secs: 3600,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        db: Arc::new(Db { healthy: true }),
    };

    let app = App::with_state(state)
        .meta(AppMeta::new("Flowgate Groups Demo", "0.2.0"))
        .pre(RequestIdMiddleware)
        .get("/health", health)?
        .group(
            Group::new("/api/v1")
                .tag("api")
                .layer(TimeoutMiddleware::new(Duration::from_secs(30)))
                .get("/users/{id}", get_user)
                .post("/users", create_user)
                .group(
                    Group::new("/admin")
                        .tag("admin")
                        .get("/stats", admin_stats),
                ),
        )
        .layer(TracingMiddleware);

    let config = ServerConfig::from_env();

    flowgate::server::serve(app, config).await?;
    Ok(())
}
