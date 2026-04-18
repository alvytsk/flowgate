//! HTTPS example. Two cert-loading paths:
//!
//! 1. **In-memory self-signed** (default): generates a fresh cert via `rcgen` at startup.
//! 2. **PEM files from disk**: set `FLOWGATE_CERT` and `FLOWGATE_KEY` to PEM paths.
//!
//! Run with: `cargo run --example tls --features tls`
//! Then:    `curl -k https://localhost:8443/`
//!
//! File-based example:
//! ```sh
//! openssl req -x509 -newkey rsa:2048 -sha256 -days 365 -nodes \
//!   -keyout /tmp/key.pem -out /tmp/cert.pem \
//!   -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
//! FLOWGATE_CERT=/tmp/cert.pem FLOWGATE_KEY=/tmp/key.pem \
//!   cargo run --example tls --features tls
//! ```

use std::sync::Arc;

use flowgate::extract::state::State;
use flowgate::{App, ServerConfig, TlsConfig};
use rustls::ServerConfig as RustlsServerConfig;

#[derive(Clone)]
struct AppState {
    greeting: String,
}

async fn hello(State(state): State<AppState>) -> String {
    format!("{} over TLS!\n", state.greeting)
}

fn load_tls() -> Result<TlsConfig, Box<dyn std::error::Error>> {
    match (std::env::var("FLOWGATE_CERT"), std::env::var("FLOWGATE_KEY")) {
        (Ok(cert), Ok(key)) => {
            println!("loading TLS from files: cert={cert} key={key}");
            Ok(TlsConfig::from_pem_files(cert, key)?)
        }
        _ => {
            println!("generating self-signed cert in memory (set FLOWGATE_CERT/FLOWGATE_KEY to use a file)");
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])?;
            let cert_der = cert.cert.der().clone();
            let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
            let rustls_cfg = RustlsServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![cert_der], key_der.into())?;
            Ok(TlsConfig::from_rustls(Arc::new(rustls_cfg)))
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tls = load_tls()?;

    let state = AppState {
        greeting: "hello".to_owned(),
    };

    let app = App::with_state(state).get("/", hello)?;

    let config = ServerConfig::new().host("127.0.0.1").port(8443).tls(tls);

    println!("serving HTTPS on https://localhost:8443 (self-signed — curl with -k)");

    let _handle = flowgate::server::serve(app, config).await?;
    std::future::pending::<()>().await;
    Ok(())
}
