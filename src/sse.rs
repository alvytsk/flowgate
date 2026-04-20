//! Server-Sent Events (SSE) response streaming.
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//! use flowgate::sse::{Event, Sse};
//! use futures_util::stream;
//!
//! async fn events() -> Sse<impl futures_core::Stream<Item = Event>> {
//!     let events = stream::iter((0..).map(|n| Event::default().data(format!("tick {n}"))));
//!     Sse::new(events).keep_alive(Duration::from_secs(15))
//! }
//! ```
//!
//! The `IntoResponse` impl sets:
//! - `Content-Type: text/event-stream`
//! - `Cache-Control: no-cache`
//! - `X-Accel-Buffering: no` (disables reverse-proxy buffering, e.g. nginx)
//!
//! When `keep_alive(..)` is set, an SSE comment frame (`:\n\n`) is emitted on
//! every tick. Comment frames are ignored by clients per the SSE spec but
//! keep the socket warm for intermediaries that idle out quiet streams.
//! The response ends as soon as the user stream ends — the heartbeat does not
//! keep the connection open past that point.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_core::Stream;
use futures_util::stream::StreamExt;
use http::HeaderValue;

use crate::body::{stream as body_stream, Response};
use crate::response::IntoResponse;

/// One server-sent event.
///
/// Built via chainable methods; each field maps to a line in the wire format.
/// An event with no fields serializes to a single blank line.
#[derive(Debug, Default, Clone)]
pub struct Event {
    data: Option<String>,
    event: Option<String>,
    id: Option<String>,
    retry: Option<Duration>,
}

impl Event {
    /// Event payload (`data:` lines). Newlines inside the payload produce
    /// multiple `data:` lines, per the SSE spec.
    pub fn data(mut self, data: impl Into<String>) -> Self {
        self.data = Some(data.into());
        self
    }

    /// Custom event name (`event:` line).
    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }

    /// Event identifier (`id:` line). Clients store it and send it back
    /// as `Last-Event-ID` on reconnect.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Client reconnect delay (`retry:` line), in milliseconds on the wire.
    pub fn retry(mut self, retry: Duration) -> Self {
        self.retry = Some(retry);
        self
    }

    /// Serialize to the SSE wire format.
    fn to_bytes(&self) -> Bytes {
        let mut buf = String::new();
        if let Some(event) = &self.event {
            for line in event.split('\n') {
                buf.push_str("event: ");
                buf.push_str(line);
                buf.push('\n');
            }
        }
        if let Some(id) = &self.id {
            for line in id.split('\n') {
                buf.push_str("id: ");
                buf.push_str(line);
                buf.push('\n');
            }
        }
        if let Some(retry) = &self.retry {
            buf.push_str("retry: ");
            buf.push_str(&retry.as_millis().to_string());
            buf.push('\n');
        }
        if let Some(data) = &self.data {
            for line in data.split('\n') {
                buf.push_str("data: ");
                buf.push_str(line);
                buf.push('\n');
            }
        }
        buf.push('\n');
        Bytes::from(buf)
    }
}

/// SSE response wrapper around a stream of events.
pub struct Sse<S> {
    stream: S,
    keep_alive: Option<Duration>,
}

impl<S> Sse<S>
where
    S: Stream<Item = Event> + Send + 'static,
{
    /// Wrap a stream of events.
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            keep_alive: None,
        }
    }

    /// Interleave a comment frame (`:\n\n`) on every tick. Recommended for
    /// long-lived streams behind reverse proxies that buffer quiet sockets.
    pub fn keep_alive(mut self, interval: Duration) -> Self {
        self.keep_alive = Some(interval);
        self
    }
}

impl<S> IntoResponse for Sse<S>
where
    S: Stream<Item = Event> + Send + 'static,
{
    fn into_response(self) -> Response {
        let events: DynChunkStream = Box::pin(
            self.stream
                .map(|event| Ok::<_, Infallible>(event.to_bytes())),
        );

        let body = match self.keep_alive {
            Some(interval) => {
                let heartbeat: DynChunkStream = Box::pin(heartbeat_stream(interval));
                body_stream(SseStream {
                    events,
                    heartbeat: Some(heartbeat),
                })
            }
            None => body_stream(events),
        };

        let mut res = http::Response::new(body);
        let headers = res.headers_mut();
        headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            http::header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        );
        headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
        res
    }
}

type DynChunkStream = Pin<Box<dyn Stream<Item = Result<Bytes, Infallible>> + Send>>;

/// Merges events and heartbeat; ends the moment the events stream ends.
/// The heartbeat alone can never keep the response open.
struct SseStream {
    events: DynChunkStream,
    heartbeat: Option<DynChunkStream>,
}

impl Stream for SseStream {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;

        match this.events.as_mut().poll_next(cx) {
            Poll::Ready(Some(item)) => return Poll::Ready(Some(item)),
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        if let Some(hb) = this.heartbeat.as_mut() {
            match hb.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) => return Poll::Ready(Some(item)),
                Poll::Ready(None) => this.heartbeat = None,
                Poll::Pending => {}
            }
        }

        Poll::Pending
    }
}

/// Unbounded stream emitting an SSE comment frame on every `interval` tick.
fn heartbeat_stream(interval: Duration) -> impl Stream<Item = Result<Bytes, Infallible>> + Send {
    struct Heartbeat {
        interval: tokio::time::Interval,
    }

    impl Stream for Heartbeat {
        type Item = Result<Bytes, Infallible>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            match self.interval.poll_tick(cx) {
                Poll::Ready(_) => Poll::Ready(Some(Ok(Bytes::from_static(b":\n\n")))),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    let mut ticker = tokio::time::interval(interval);
    // Skip the instant first-tick; we want the first heartbeat after a full interval.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.reset_after(interval);

    Heartbeat { interval: ticker }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_data_only() {
        let out = Event::default().data("hello").to_bytes();
        assert_eq!(&out[..], b"data: hello\n\n");
    }

    #[test]
    fn event_multiline_data() {
        let out = Event::default().data("line one\nline two").to_bytes();
        assert_eq!(&out[..], b"data: line one\ndata: line two\n\n");
    }

    #[test]
    fn event_full_fields_order() {
        let out = Event::default()
            .event("ping")
            .id("42")
            .retry(Duration::from_millis(2500))
            .data("payload")
            .to_bytes();
        assert_eq!(
            std::str::from_utf8(&out).unwrap(),
            "event: ping\nid: 42\nretry: 2500\ndata: payload\n\n"
        );
    }

    #[test]
    fn event_empty_serializes_blank_line() {
        let out = Event::default().to_bytes();
        assert_eq!(&out[..], b"\n");
    }
}
