//! WebSocket echo server.
//!
//! Run with: `cargo run --example ws_echo --features ws`
//! Then:    `websocat ws://localhost:8080/ws`
//!
//! Every text or binary message the client sends is echoed back verbatim.

use flowgate::body::Response;
use flowgate::ws::WebSocketUpgrade;
use flowgate::{App, ServerConfig};

async fn ws_handler(upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(|mut socket| async move {
        while let Some(Ok(msg)) = socket.recv().await {
            if msg.is_close() {
                break;
            }
            if socket.send(msg).await.is_err() {
                break;
            }
        }
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::new().get("/ws", ws_handler)?;

    let config = ServerConfig::new().host("127.0.0.1").port(8080);

    println!("WebSocket echo on ws://localhost:8080/ws");
    println!("try: websocat ws://localhost:8080/ws");

    let _handle = flowgate::server::serve(app, config).await?;
    std::future::pending::<()>().await;
    Ok(())
}
