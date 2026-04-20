use std::convert::Infallible;
use std::env;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

#[derive(Deserialize, Serialize)]
struct EchoBody {
    name: String,
    id: i64,
}

async fn handle(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/plaintext") => Ok(Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/plain; charset=utf-8")
            .body(Full::new(Bytes::from_static(b"Hello, World!")))
            .unwrap()),
        (&Method::POST, "/echo") => {
            let body = req.into_body().collect().await;
            let bytes = match body {
                Ok(b) => b.to_bytes(),
                Err(_) => {
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Full::default())
                        .unwrap());
                }
            };
            let parsed: Result<EchoBody, _> = serde_json::from_slice(&bytes);
            match parsed {
                Ok(echo) => {
                    let out = serde_json::to_vec(&echo).unwrap();
                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/json")
                        .body(Full::new(Bytes::from(out)))
                        .unwrap())
                }
                Err(_) => Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::default())
                    .unwrap()),
            }
        }
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::default())
            .unwrap()),
    }
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
        let listener = TcpListener::bind(("127.0.0.1", port)).await?;
        loop {
            let (stream, _) = listener.accept().await?;
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let _ = http1::Builder::new()
                    .serve_connection(io, service_fn(handle))
                    .await;
            });
        }
        #[allow(unreachable_code)]
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}
