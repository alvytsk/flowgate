//! Request observation hook for metrics backends.
//!
//! `MetricsObserver` fires once per request, after the response has been
//! produced. It is deliberately keyed on the **matched route pattern** (e.g.
//! `/users/{id}`) — never the raw path — so downstream metrics stores stay
//! bounded in cardinality.
//!
//! Observers registered via [`crate::App::observe`] are invoked from the
//! dispatch path; an empty observer list stays on the fast path (no wall-clock
//! reads, no boxed futures).

use std::time::Duration;

use http::{Method, StatusCode};

/// Event emitted once per request after the response is produced.
///
/// Borrowed — observers receive it by reference and should not hold on to
/// any of its fields beyond the call.
pub struct RequestEvent<'a> {
    /// HTTP method of the request.
    pub method: &'a Method,
    /// Matched route pattern (e.g. `/users/{id}`). `None` for 404 responses.
    /// For 405, the pattern is also `None` — no specific handler matched.
    pub route_pattern: Option<&'a str>,
    /// Final response status code.
    pub status: StatusCode,
    /// Wall-clock time spent inside the framework, measured from just before
    /// route matching to just after the response is assembled.
    pub duration: Duration,
}

/// Observer trait — implement this to record metrics for each request.
///
/// Implementations must be cheap and non-blocking; the observer is called
/// synchronously on the hot path. For any I/O (pushing to a remote backend,
/// writing to a file), hand the event off to a background task via a channel.
pub trait MetricsObserver: Send + Sync {
    fn on_request(&self, event: &RequestEvent<'_>);
}
