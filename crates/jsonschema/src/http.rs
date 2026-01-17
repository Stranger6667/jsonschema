//! HTTP client configuration for schema retrieval.
use std::{path::PathBuf, time::Duration};

/// Configuration options for HTTP client used in schema retrieval.
///
/// This struct provides builder-style methods to configure the HTTP client
/// used when fetching external schemas via HTTP/HTTPS.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
/// use jsonschema::HttpOptions;
///
/// let http_options = HttpOptions::new()
///     .connect_timeout(Duration::from_secs(10))
///     .timeout(Duration::from_secs(30))
///     .danger_accept_invalid_certs(false);
/// ```
#[derive(Debug, Clone, Default)]
pub struct HttpOptions {
    pub(crate) connect_timeout: Option<Duration>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) danger_accept_invalid_certs: bool,
    pub(crate) root_certificate: Option<PathBuf>,
}

impl HttpOptions {
    /// Create a new `HttpOptions` with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the timeout for the connect phase of a connection.
    ///
    /// This controls how long the client will wait to establish a connection
    /// to the remote server.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use jsonschema::HttpOptions;
    ///
    /// let options = HttpOptions::new()
    ///     .connect_timeout(Duration::from_secs(10));
    /// ```
    #[must_use]
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Set the total timeout for the entire request.
    ///
    /// This includes connection time, any redirects, and reading the response body.
    ///
    /// **Note**: If `timeout` is smaller than `connect_timeout`, the total timeout
    /// takes precedence and will cap the connection attempt.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use jsonschema::HttpOptions;
    ///
    /// let options = HttpOptions::new()
    ///     .timeout(Duration::from_secs(60));
    /// ```
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Controls whether to accept invalid TLS certificates.
    ///
    /// **WARNING**: Setting this to `true` disables certificate validation,
    /// which makes the connection vulnerable to man-in-the-middle attacks.
    /// Only use this for testing or in controlled environments.
    ///
    /// # Example
    ///
    /// ```rust
    /// use jsonschema::HttpOptions;
    ///
    /// // Not recommended for production use!
    /// let options = HttpOptions::new()
    ///     .danger_accept_invalid_certs(true);
    /// ```
    #[must_use]
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.danger_accept_invalid_certs = accept;
        self
    }

    /// Add a custom root certificate for TLS verification.
    ///
    /// This allows you to trust a custom CA certificate in addition to
    /// the system's root certificates.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::PathBuf;
    /// use jsonschema::HttpOptions;
    ///
    /// let options = HttpOptions::new()
    ///     .add_root_certificate("/path/to/ca-cert.pem");
    /// ```
    #[must_use]
    pub fn add_root_certificate(mut self, path: impl Into<PathBuf>) -> Self {
        self.root_certificate = Some(path.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_methods() {
        let options = HttpOptions::new()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .danger_accept_invalid_certs(true)
            .add_root_certificate("/path/to/cert.pem");

        assert_eq!(options.connect_timeout, Some(Duration::from_secs(10)));
        assert_eq!(options.timeout, Some(Duration::from_secs(30)));
        assert!(options.danger_accept_invalid_certs);
        assert_eq!(
            options.root_certificate,
            Some(PathBuf::from("/path/to/cert.pem"))
        );
    }
}
