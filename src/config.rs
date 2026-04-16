use std::time::Duration;

/// Server configuration with embedded-safe defaults.
pub struct ServerConfig {
    /// Host to bind to (e.g. "0.0.0.0", "127.0.0.1").
    pub host: String,
    /// Port to bind to. Default: 8080.
    pub port: u16,
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
            host: "0.0.0.0".to_owned(),
            port: 8080,
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

    /// Create config reading `HOST` and `PORT` from environment variables.
    /// Falls back to defaults (`0.0.0.0:8080`) for unset or invalid values.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(host) = std::env::var("HOST") {
            config.host = host;
        }
        if let Ok(port) = std::env::var("PORT") {
            if let Ok(port) = port.parse::<u16>() {
                config.port = port;
            }
        }
        config
    }

    /// Return the bind address as `"host:port"`.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Set the full bind address (e.g. `"0.0.0.0:8080"`).
    /// Parses into host and port. If parsing fails, treats the
    /// entire string as the host and keeps the current port.
    pub fn addr(mut self, addr: impl Into<String>) -> Self {
        let addr = addr.into();
        if let Some((host, port_str)) = addr.rsplit_once(':') {
            if let Ok(port) = port_str.parse::<u16>() {
                self.host = host.to_owned();
                self.port = port;
                return self;
            }
        }
        self.host = addr;
        self
    }

    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
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
