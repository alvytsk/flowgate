use std::env;

use flowgate::extract::json::Json;
use flowgate::{App, ServerConfig};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct EchoBody {
    name: String,
    id: i64,
}

async fn plaintext() -> &'static str {
    "Hello, World!"
}

async fn echo(Json(body): Json<EchoBody>) -> Json<EchoBody> {
    Json(body)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workers: usize = env::var("BENCH_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let port: u16 = env::var("BENCH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);

    let mut builder = if workers <= 1 {
        tokio::runtime::Builder::new_current_thread()
    } else {
        let mut b = tokio::runtime::Builder::new_multi_thread();
        b.worker_threads(workers);
        b
    };
    let runtime = builder.enable_all().build()?;

    runtime.block_on(async move {
        let app = App::new()
            .get("/plaintext", plaintext)?
            .post("/echo", echo)?;

        let config = ServerConfig::default()
            .host("127.0.0.1")
            .port(port)
            .enable_default_tracing(false);

        let _handle = flowgate::server::serve(app, config).await?;
        std::future::pending::<()>().await;
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}
