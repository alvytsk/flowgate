//! WebSocket support.
//!
//! # Example
//!
//! ```no_run
//! use flowgate::{App, ServerConfig};
//! use flowgate::body::Response;
//! use flowgate::ws::{Message, WebSocketUpgrade};
//!
//! async fn echo(ws: WebSocketUpgrade) -> Response {
//!     ws.on_upgrade(|mut socket| async move {
//!         while let Some(Ok(msg)) = socket.recv().await {
//!             if socket.send(msg).await.is_err() {
//!                 break;
//!             }
//!         }
//!     })
//! }
//! ```
//!
//! Handshake validation is token-aware (case-insensitive, comma-separated):
//!
//! - `Connection` must contain the token `upgrade`
//! - `Upgrade` must contain the token `websocket`
//! - `Sec-WebSocket-Version` must equal `13`
//! - `Sec-WebSocket-Key` must base64-decode to exactly 16 bytes
//!
//! The upgrade task is spawned **detached** on the tokio runtime — see
//! [`ServerHandle::shutdown`](crate::ServerHandle::shutdown) for the
//! shutdown-semantics carve-out.

use std::future::Future;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use http::request::Parts;
use http::StatusCode;
use hyper::upgrade::{OnUpgrade, Upgraded};
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::WebSocketStream;

pub use tokio_tungstenite::tungstenite::Message;

use crate::body::{empty, Request, Response};
use crate::extract::{FromRequest, FromRequestParts};
use crate::response::IntoResponse;

const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Extractor that accepts an HTTP Upgrade request for the WebSocket protocol.
///
/// Use [`on_upgrade`](Self::on_upgrade) to return the 101 response and spawn
/// a task that drives the WebSocket session.
pub struct WebSocketUpgrade {
    on_upgrade: OnUpgrade,
    sec_accept: HeaderValue,
}

impl<S: Send + Sync> FromRequestParts<S> for WebSocketUpgrade {
    type Rejection = WsError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if !header_contains_token(&parts.headers, &http::header::CONNECTION, "upgrade") {
            return Err(WsError::MissingConnectionUpgrade);
        }
        if !header_contains_token(&parts.headers, &http::header::UPGRADE, "websocket") {
            return Err(WsError::MissingUpgradeWebsocket);
        }

        let version = parts
            .headers
            .get(http::header::SEC_WEBSOCKET_VERSION)
            .ok_or(WsError::MissingVersion)?;
        if version.as_bytes() != b"13" {
            return Err(WsError::UnsupportedVersion);
        }

        let key = parts
            .headers
            .get(http::header::SEC_WEBSOCKET_KEY)
            .ok_or(WsError::MissingKey)?;

        // Validate: base64 of exactly 16 bytes.
        let decoded = B64
            .decode(key.as_bytes())
            .map_err(|_| WsError::InvalidKey)?;
        if decoded.len() != 16 {
            return Err(WsError::InvalidKey);
        }

        let sec_accept = sec_websocket_accept(key.as_bytes());

        let on_upgrade = parts
            .extensions
            .remove::<OnUpgrade>()
            .ok_or(WsError::MissingOnUpgrade)?;

        Ok(Self {
            on_upgrade,
            sec_accept,
        })
    }
}

/// Also impl `FromRequest` so `WebSocketUpgrade` can stand alone as the only
/// handler argument (which uses `FromRequest`).
impl<S: Send + Sync> FromRequest<S> for WebSocketUpgrade {
    type Rejection = WsError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, _body) = req.into_parts();
        <Self as FromRequestParts<S>>::from_request_parts(&mut parts, state).await
    }
}

impl WebSocketUpgrade {
    /// Return a 101 Switching Protocols response and spawn a detached task
    /// that awaits the upgrade, constructs a [`WebSocket`], and invokes the
    /// provided callback.
    ///
    /// The spawned task survives `ServerHandle::shutdown` — see its rustdoc
    /// for the shutdown carve-out. Applications should use their own
    /// shutdown signal to close active sessions cleanly.
    pub fn on_upgrade<F, Fut>(self, callback: F) -> Response
    where
        F: FnOnce(WebSocket) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let Self {
            on_upgrade,
            sec_accept,
        } = self;

        tokio::spawn(async move {
            match on_upgrade.await {
                Ok(upgraded) => {
                    let io = TokioIo::new(upgraded);
                    let ws_stream =
                        WebSocketStream::from_raw_socket(io, Role::Server, None).await;
                    callback(WebSocket { inner: ws_stream }).await;
                }
                Err(err) => {
                    tracing::warn!("websocket upgrade failed: {err}");
                }
            }
        });

        http::Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(http::header::UPGRADE, HeaderValue::from_static("websocket"))
            .header(http::header::CONNECTION, HeaderValue::from_static("upgrade"))
            .header(http::header::SEC_WEBSOCKET_ACCEPT, sec_accept)
            .body(empty())
            .expect("101 response is always well-formed")
    }
}

/// Upgraded WebSocket session — thin wrapper over `tokio_tungstenite::WebSocketStream`.
pub struct WebSocket {
    inner: WebSocketStream<TokioIo<Upgraded>>,
}

impl WebSocket {
    /// Receive the next message. Returns `None` once the peer has closed.
    pub async fn recv(&mut self) -> Option<Result<Message, WsError>> {
        self.inner
            .next()
            .await
            .map(|res| res.map_err(WsError::Protocol))
    }

    /// Send a message to the peer.
    pub async fn send(&mut self, msg: Message) -> Result<(), WsError> {
        self.inner.send(msg).await.map_err(WsError::Protocol)
    }

    /// Initiate a WebSocket close handshake.
    pub async fn close(&mut self) -> Result<(), WsError> {
        self.inner.close(None).await.map_err(WsError::Protocol)
    }
}

/// Errors produced by the WebSocket upgrade extractor and [`WebSocket`].
#[derive(Debug)]
pub enum WsError {
    /// Request did not carry `Connection: upgrade` (token-aware).
    MissingConnectionUpgrade,
    /// Request did not carry `Upgrade: websocket` (token-aware).
    MissingUpgradeWebsocket,
    /// `Sec-WebSocket-Version` header was absent.
    MissingVersion,
    /// `Sec-WebSocket-Version` was present but not `13`.
    UnsupportedVersion,
    /// `Sec-WebSocket-Key` header was absent.
    MissingKey,
    /// `Sec-WebSocket-Key` was present but did not base64-decode to 16 bytes.
    InvalidKey,
    /// Hyper did not install an `OnUpgrade` extension on the request — likely
    /// misconfigured server setup.
    MissingOnUpgrade,
    /// Protocol-level error from the underlying WebSocket stream.
    Protocol(tokio_tungstenite::tungstenite::Error),
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingConnectionUpgrade => write!(f, "missing `Connection: upgrade`"),
            Self::MissingUpgradeWebsocket => write!(f, "missing `Upgrade: websocket`"),
            Self::MissingVersion => write!(f, "missing `Sec-WebSocket-Version`"),
            Self::UnsupportedVersion => write!(f, "unsupported WebSocket version (need 13)"),
            Self::MissingKey => write!(f, "missing `Sec-WebSocket-Key`"),
            Self::InvalidKey => write!(f, "invalid `Sec-WebSocket-Key`"),
            Self::MissingOnUpgrade => write!(f, "missing hyper OnUpgrade extension"),
            Self::Protocol(err) => write!(f, "websocket protocol error: {err}"),
        }
    }
}

impl std::error::Error for WsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Protocol(err) => Some(err),
            _ => None,
        }
    }
}

impl IntoResponse for WsError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, self.to_string()).into_response()
    }
}

/// Case-insensitive, comma-token-aware header contains check.
///
/// `Connection: keep-alive, Upgrade` contains the token `upgrade`; naive
/// string equality would reject this and break legitimate clients.
fn header_contains_token(headers: &HeaderMap, name: &HeaderName, token: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .flat_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|part| part.trim().eq_ignore_ascii_case(token))
}

fn sec_websocket_accept(key: &[u8]) -> HeaderValue {
    let mut hasher = Sha1::new();
    hasher.update(key);
    hasher.update(WS_GUID);
    let digest = hasher.finalize();
    let encoded = B64.encode(digest);
    HeaderValue::from_str(&encoded).expect("base64 output is valid header value")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_websocket_accept_rfc_6455_example() {
        // RFC 6455 §1.3: key "dGhlIHNhbXBsZSBub25jZQ==" -> "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        let accept = sec_websocket_accept(b"dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(accept.to_str().unwrap(), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn header_contains_token_simple() {
        let mut headers = HeaderMap::new();
        headers.insert(http::header::CONNECTION, HeaderValue::from_static("Upgrade"));
        assert!(header_contains_token(
            &headers,
            &http::header::CONNECTION,
            "upgrade"
        ));
    }

    #[test]
    fn header_contains_token_compound() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONNECTION,
            HeaderValue::from_static("keep-alive, Upgrade"),
        );
        assert!(header_contains_token(
            &headers,
            &http::header::CONNECTION,
            "upgrade"
        ));
    }

    #[test]
    fn header_contains_token_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(http::header::UPGRADE, HeaderValue::from_static("WebSocket"));
        assert!(header_contains_token(
            &headers,
            &http::header::UPGRADE,
            "websocket"
        ));
    }

    #[test]
    fn header_contains_token_negative() {
        let mut headers = HeaderMap::new();
        headers.insert(http::header::CONNECTION, HeaderValue::from_static("close"));
        assert!(!header_contains_token(
            &headers,
            &http::header::CONNECTION,
            "upgrade"
        ));
    }
}
