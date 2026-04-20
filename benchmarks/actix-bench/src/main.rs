use std::env;

use actix_web::{post, web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct EchoBody {
    name: String,
    id: i64,
}

async fn plaintext() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body("Hello, World!")
}

#[post("/echo")]
async fn echo(body: web::Json<EchoBody>) -> impl Responder {
    web::Json(body.into_inner())
}

fn main() -> std::io::Result<()> {
    let workers: usize = env::var("BENCH_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let port: u16 = env::var("BENCH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);

    actix_web::rt::System::new().block_on(async move {
        HttpServer::new(|| {
            App::new()
                .route("/plaintext", web::get().to(plaintext))
                .service(echo)
        })
        .workers(workers)
        .bind(("127.0.0.1", port))?
        .run()
        .await
    })
}
