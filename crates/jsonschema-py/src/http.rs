use std::hash::{Hash, Hasher};

use pyo3::{exceptions::PyValueError, prelude::*};

fn validate_timeout(value: Option<f64>, name: &str) -> PyResult<Option<f64>> {
    match value {
        Some(v) if v < 0.0 || v.is_nan() || v.is_infinite() => Err(PyValueError::new_err(format!(
            "{name} must be a non-negative finite number"
        ))),
        _ => Ok(value),
    }
}

/// HttpOptions(timeout=None, connect_timeout=None, tls_verify=True, ca_cert=None)
///
/// Configuration for HTTP client used when fetching external schemas.
///
/// This allows customization of HTTP request behavior when validating schemas
/// that reference external resources via HTTP/HTTPS.
///
///     >>> from jsonschema_rs import validator_for, HttpOptions
///     >>> validator = validator_for(
///     ...     {"$ref": "https://example.com/schema.json"},
///     ...     http_options=HttpOptions(timeout=30.0, connect_timeout=10.0)
///     ... )
///
/// Parameters:
///     timeout: Total request timeout in seconds (default: None - no timeout)
///     connect_timeout: Connection timeout in seconds (default: None - no timeout)
///     tls_verify: Whether to verify TLS certificates (default: True)
///     ca_cert: Path to a custom CA certificate file in PEM format (default: None)
#[pyclass(module = "jsonschema_rs", get_all)]
#[derive(Clone)]
pub(crate) struct HttpOptions {
    pub(crate) timeout: Option<f64>,
    pub(crate) connect_timeout: Option<f64>,
    pub(crate) tls_verify: bool,
    pub(crate) ca_cert: Option<String>,
}

#[pymethods]
impl HttpOptions {
    #[new]
    #[pyo3(signature = (timeout=None, connect_timeout=None, tls_verify=true, ca_cert=None))]
    fn new(
        timeout: Option<f64>,
        connect_timeout: Option<f64>,
        tls_verify: bool,
        ca_cert: Option<String>,
    ) -> PyResult<Self> {
        Ok(Self {
            timeout: validate_timeout(timeout, "timeout")?,
            connect_timeout: validate_timeout(connect_timeout, "connect_timeout")?,
            tls_verify,
            ca_cert,
        })
    }

    fn __repr__(&self) -> String {
        use std::fmt::Write;
        let mut s = String::from("HttpOptions(timeout=");
        match self.timeout {
            Some(t) => write!(s, "{t}"),
            None => write!(s, "None"),
        }
        .expect("Failed to write timeout");
        s.push_str(", connect_timeout=");
        match self.connect_timeout {
            Some(t) => write!(s, "{t}"),
            None => write!(s, "None"),
        }
        .expect("Failed to write connect_timeout");
        s.push_str(", tls_verify=");
        s.push_str(if self.tls_verify { "True" } else { "False" });
        s.push_str(", ca_cert=");
        match &self.ca_cert {
            Some(c) => write!(s, "'{c}'"),
            None => write!(s, "None"),
        }
        .expect("Failed to write ca_cert");
        s.push(')');
        s
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other) = other.extract::<Self>() {
            self.timeout == other.timeout
                && self.connect_timeout == other.connect_timeout
                && self.tls_verify == other.tls_verify
                && self.ca_cert == other.ca_cert
        } else {
            false
        }
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.timeout.map(f64::to_bits).hash(&mut hasher);
        self.connect_timeout.map(f64::to_bits).hash(&mut hasher);
        self.tls_verify.hash(&mut hasher);
        self.ca_cert.hash(&mut hasher);
        hasher.finish()
    }
}
