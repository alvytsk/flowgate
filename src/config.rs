use std::time::Duration;

/// Server configuration with embedded-safe defaults.
pub struct ServerConfig {
    /// Address to bind to (e.g. "0.0.0.0:8080").
    pub addr: String,
    /// Maximum JSON body size in bytes. Default: 256 KiB.
    pub json_body_limit: usize,
    /// Enable HTTP/1.1 keep-alive. Default: true.
    pub keep_alive: bool,
    /// Header read timeout. Requires hyper-util TokioTimer.
    pub header_read_timeout: Option<Duration>,
    /// Maximum number of headers. Useful for embedded targets.
    pub max_headers: Option<usize>,
    /// Enable the default tracing subscriber. Default: true.
    pub enable_default_tracing: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: "0.0.0.0:8080".to_owned(),
            json_body_limit: 262_144, // 256 KiB
            keep_alive: true,
            header_read_timeout: Some(Duration::from_secs(5)),
            max_headers: Some(64),
            enable_default_tracing: true,
        }
    }
}

impl ServerConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn addr(mut self, addr: impl Into<String>) -> Self {
        self.addr = addr.into();
        self
    }

    pub fn json_body_limit(mut self, limit: usize) -> Self {
        self.json_body_limit = limit;
        self
    }

    pub fn keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    pub fn header_read_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.header_read_timeout = timeout;
        self
    }

    pub fn max_headers(mut self, max: Option<usize>) -> Self {
        self.max_headers = max;
        self
    }

    pub fn enable_default_tracing(mut self, enable: bool) -> Self {
        self.enable_default_tracing = enable;
        self
    }
}
