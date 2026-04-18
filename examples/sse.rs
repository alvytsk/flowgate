//! SSE (Server-Sent Events) example.
//!
//! Run with: `cargo run --example sse`
//! Then:    `curl -N http://localhost:8080/events`
//!
//! The endpoint emits one `data: tick N` event per second, indefinitely, with
//! a 15s keep-alive comment frame to keep reverse proxies from idling out.

use std::time::Duration;

use futures_util::stream;
use futures_util::StreamExt;

use flowgate::sse::{Event, Sse};
use flowgate::{App, ServerConfig};

async fn events() -> Sse<impl futures_core::Stream<Item = Event>> {
    let ticker = stream::unfold(0u64, |n| async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Some((Event::default().id(n.to_string()).data(format!("tick {n}")), n + 1))
    });
    Sse::new(ticker.boxed()).keep_alive(Duration::from_secs(15))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::new().get("/events", events)?;

    let config = ServerConfig::new().host("127.0.0.1").port(8080);

    println!("streaming SSE on http://localhost:8080/events");
    println!("try: curl -N http://localhost:8080/events");

    let _handle = flowgate::server::serve(app, config).await?;
    std::future::pending::<()>().await;
    Ok(())
}
