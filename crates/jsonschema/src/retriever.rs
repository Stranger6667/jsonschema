//! Logic for retrieving external resources.
use referencing::{Retrieve, Uri};
use serde_json::Value;

#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
use crate::HttpOptions;

/// Error that can occur when creating an HTTP retriever.
#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
#[derive(Debug)]
pub enum HttpRetrieverError {
    /// Failed to read certificate file.
    CertificateRead {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    /// Failed to parse certificate.
    CertificateParse {
        path: std::path::PathBuf,
        source: reqwest::Error,
    },
    /// Failed to build HTTP client.
    ClientBuild(reqwest::Error),
}

#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
impl std::fmt::Display for HttpRetrieverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CertificateRead { path, source } => {
                write!(
                    f,
                    "Failed to read certificate file '{}': {source}",
                    path.display()
                )
            }
            Self::CertificateParse { path, source } => {
                write!(
                    f,
                    "Failed to parse certificate '{}': {source}",
                    path.display()
                )
            }
            Self::ClientBuild(e) => write!(f, "Failed to build HTTP client: {e}"),
        }
    }
}

#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
impl std::error::Error for HttpRetrieverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CertificateRead { source, .. } => Some(source),
            Self::CertificateParse { source, .. } | Self::ClientBuild(source) => Some(source),
        }
    }
}

/// Load a certificate from a PEM file.
#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
fn load_certificate(path: &std::path::Path) -> Result<reqwest::Certificate, HttpRetrieverError> {
    let cert_data = std::fs::read(path).map_err(|e| HttpRetrieverError::CertificateRead {
        path: path.to_path_buf(),
        source: e,
    })?;
    reqwest::Certificate::from_pem(&cert_data).map_err(|e| HttpRetrieverError::CertificateParse {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Configure an HTTP client builder with the given options.
/// Works with both `reqwest::ClientBuilder` and `reqwest::blocking::ClientBuilder`.
#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
macro_rules! configure_http_client {
    ($builder:expr, $options:expr) => {{
        let mut builder = $builder;
        if let Some(connect_timeout) = $options.connect_timeout {
            builder = builder.connect_timeout(connect_timeout);
        }
        if let Some(timeout) = $options.timeout {
            builder = builder.timeout(timeout);
        }
        if $options.danger_accept_invalid_certs {
            builder = builder.danger_accept_invalid_certs(true);
        }
        if let Some(ref cert_path) = &$options.root_certificate {
            builder = builder.add_root_certificate(load_certificate(cert_path)?);
        }
        builder
    }};
}

pub(crate) struct DefaultRetriever;

/// HTTP-based schema retriever with configurable client options.
///
/// This retriever fetches external schemas over HTTP/HTTPS using a
/// configured [`reqwest`](https://docs.rs/reqwest) client.
///
/// # Example
///
/// ```rust,no_run
/// use std::time::Duration;
/// use jsonschema::{HttpOptions, HttpRetriever};
///
/// let http_options = HttpOptions::new()
///     .timeout(Duration::from_secs(30));
///
/// let retriever = HttpRetriever::new(&http_options)
///     .expect("Failed to create HTTP retriever");
/// ```
#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
#[derive(Debug)]
pub struct HttpRetriever {
    client: reqwest::blocking::Client,
}

#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
impl HttpRetriever {
    /// Create a new HTTP retriever with the given options.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The certificate file cannot be read
    /// - The certificate is not valid PEM
    /// - The HTTP client cannot be built
    pub fn new(options: &HttpOptions) -> Result<Self, HttpRetrieverError> {
        // Install ring as the default TLS crypto provider (only needed once)
        let _ = rustls::crypto::ring::default_provider().install_default();
        let builder = configure_http_client!(reqwest::blocking::Client::builder(), options);
        Ok(Self {
            client: builder.build().map_err(HttpRetrieverError::ClientBuild)?,
        })
    }
}

#[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
impl Retrieve for HttpRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        match uri.scheme().as_str() {
            "http" | "https" => Ok(self.client.get(uri.as_str()).send()?.json()?),
            "file" => {
                #[cfg(feature = "resolve-file")]
                {
                    let path = uri.path().as_str();
                    let path = {
                        #[cfg(windows)]
                        {
                            let path = path.trim_start_matches('/').replace('/', "\\");
                            std::path::PathBuf::from(path)
                        }
                        #[cfg(not(windows))]
                        {
                            std::path::PathBuf::from(path)
                        }
                    };
                    let file = std::fs::File::open(path)?;
                    Ok(serde_json::from_reader(file)?)
                }
                #[cfg(not(feature = "resolve-file"))]
                {
                    Err("`resolve-file` feature or a custom resolver is required to resolve external schemas via files".into())
                }
            }
            scheme => Err(format!("Unknown scheme {scheme}").into()),
        }
    }
}

/// Async HTTP-based schema retriever with configurable client options.
///
/// This retriever fetches external schemas over HTTP/HTTPS using a
/// configured async [`reqwest`](https://docs.rs/reqwest) client.
#[cfg(all(
    feature = "resolve-http",
    feature = "resolve-async",
    not(target_arch = "wasm32")
))]
#[derive(Debug)]
pub struct AsyncHttpRetriever {
    client: reqwest::Client,
}

#[cfg(all(
    feature = "resolve-http",
    feature = "resolve-async",
    not(target_arch = "wasm32")
))]
impl AsyncHttpRetriever {
    /// Create a new async HTTP retriever with the given options.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The certificate file cannot be read
    /// - The certificate is not valid PEM
    /// - The HTTP client cannot be built
    pub fn new(options: &HttpOptions) -> Result<Self, HttpRetrieverError> {
        // Install ring as the default TLS crypto provider (only needed once)
        let _ = rustls::crypto::ring::default_provider().install_default();
        let builder = configure_http_client!(reqwest::Client::builder(), options);
        Ok(Self {
            client: builder.build().map_err(HttpRetrieverError::ClientBuild)?,
        })
    }
}

#[cfg(all(
    feature = "resolve-http",
    feature = "resolve-async",
    not(target_arch = "wasm32")
))]
#[async_trait::async_trait]
impl referencing::AsyncRetrieve for AsyncHttpRetriever {
    async fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        match uri.scheme().as_str() {
            "http" | "https" => Ok(self.client.get(uri.as_str()).send().await?.json().await?),
            "file" => {
                #[cfg(feature = "resolve-file")]
                {
                    let path = uri.path().as_str().to_string();
                    let contents = tokio::task::spawn_blocking(
                        move || -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
                            let path = {
                                #[cfg(windows)]
                                {
                                    let path = path.trim_start_matches('/').replace('/', "\\");
                                    std::path::PathBuf::from(path)
                                }
                                #[cfg(not(windows))]
                                {
                                    std::path::PathBuf::from(path)
                                }
                            };
                            let file = std::fs::File::open(path)?;
                            Ok(serde_json::from_reader(file)?)
                        },
                    )
                    .await??;
                    Ok(contents)
                }
                #[cfg(not(feature = "resolve-file"))]
                {
                    Err("`resolve-file` feature or a custom resolver is required to resolve external schemas via files".into())
                }
            }
            scheme => Err(format!("Unknown scheme {scheme}").into()),
        }
    }
}

impl Retrieve for DefaultRetriever {
    #[allow(unused)]
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            Err("External references are not supported on wasm32-unknown-unknown".into())
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        match uri.scheme().as_str() {
            "http" | "https" => {
                #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
                {
                    // Install ring as the default TLS crypto provider (only needed once)
                    let _ = rustls::crypto::ring::default_provider().install_default();
                    Ok(reqwest::blocking::get(uri.as_str())?.json()?)
                }
                #[cfg(all(feature = "resolve-http", target_arch = "wasm32"))]
                {
                    Err("Synchronous HTTP retrieval is not supported on wasm32 targets. Use async_validator_for with the resolve-async feature instead".into())
                }
                #[cfg(not(feature = "resolve-http"))]
                {
                    Err("`resolve-http` feature or a custom resolver is required to resolve external schemas via HTTP".into())
                }
            }
            "file" => {
                #[cfg(feature = "resolve-file")]
                {
                    let path = uri.path().as_str();
                    let path = {
                        #[cfg(windows)]
                        {
                            // Remove the leading slash and replace forward slashes with backslashes
                            let path = path.trim_start_matches('/').replace('/', "\\");
                            std::path::PathBuf::from(path)
                        }
                        #[cfg(not(windows))]
                        {
                            std::path::PathBuf::from(path)
                        }
                    };
                    let file = std::fs::File::open(path)?;
                    Ok(serde_json::from_reader(file)?)
                }
                #[cfg(not(feature = "resolve-file"))]
                {
                    Err("`resolve-file` feature or a custom resolver is required to resolve external schemas via files".into())
                }
            }
            scheme => Err(format!("Unknown scheme {scheme}").into()),
        }
    }
}

#[cfg(feature = "resolve-async")]
#[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
impl referencing::AsyncRetrieve for DefaultRetriever {
    async fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            Err("External references are not supported on wasm32-unknown-unknown".into())
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        match uri.scheme().as_str() {
            "http" | "https" => {
                #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
                {
                    // Install ring as the default TLS crypto provider (only needed once)
                    let _ = rustls::crypto::ring::default_provider().install_default();
                    Ok(reqwest::get(uri.as_str()).await?.json().await?)
                }
                #[cfg(all(feature = "resolve-http", target_arch = "wasm32"))]
                {
                    Ok(reqwest::get(uri.as_str()).await?.json().await?)
                }
                #[cfg(not(feature = "resolve-http"))]
                Err("`resolve-http` feature or a custom resolver is required to resolve external schemas via HTTP".into())
            }
            "file" => {
                #[cfg(feature = "resolve-file")]
                {
                    // File operations are blocking, so we use tokio's spawn_blocking
                    let path = uri.path().as_str().to_string();
                    let contents = tokio::task::spawn_blocking(
                        move || -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
                            let path = {
                                #[cfg(windows)]
                                {
                                    let path = path.trim_start_matches('/').replace('/', "\\");
                                    std::path::PathBuf::from(path)
                                }
                                #[cfg(not(windows))]
                                {
                                    std::path::PathBuf::from(path)
                                }
                            };
                            let file = std::fs::File::open(path)?;
                            Ok(serde_json::from_reader(file)?)
                        },
                    )
                    .await??;
                    Ok(contents)
                }
                #[cfg(not(feature = "resolve-file"))]
                {
                    Err("`resolve-file` feature or a custom resolver is required to resolve external schemas via files".into())
                }
            }
            scheme => Err(format!("Unknown scheme {scheme}").into()),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
use percent_encoding::{AsciiSet, CONTROLS};

#[cfg(all(test, not(target_arch = "wasm32")))]
const URI_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'#')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

#[cfg(all(test, not(target_arch = "wasm32"), not(target_os = "windows")))]
const UNIX_URI_SEGMENT: &AsciiSet = &URI_SEGMENT.add(b'\\');

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn path_to_uri(path: &std::path::Path) -> String {
    use percent_encoding::percent_encode;

    let mut result = "file://".to_owned();

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::ffi::OsStrExt;

        for component in path.components().skip(1) {
            result.push('/');
            result.extend(percent_encode(
                component.as_os_str().as_bytes(),
                UNIX_URI_SEGMENT,
            ));
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();

        match components.next() {
            Some(Component::Prefix(ref p)) => match p.kind() {
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                    result.push('/');
                    result.push(letter as char);
                    result.push(':');
                }
                _ => panic!("Unexpected path"),
            },
            _ => panic!("Unexpected path"),
        }

        for component in components {
            if component == Component::RootDir {
                continue;
            }

            let component = component.as_os_str().to_str().expect("Unexpected path");

            result.push('/');
            result.extend(percent_encode(component.as_bytes(), URI_SEGMENT));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    use super::path_to_uri;
    #[cfg(all(
        feature = "resolve-http",
        feature = "resolve-async",
        not(target_arch = "wasm32")
    ))]
    use crate::AsyncHttpRetriever;
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    use crate::{HttpOptions, HttpRetriever, HttpRetrieverError};
    use serde_json::json;
    #[cfg(not(target_arch = "wasm32"))]
    use std::io::Write;

    #[test]
    #[cfg(all(not(target_arch = "wasm32"), feature = "resolve-file"))]
    fn test_retrieve_from_file() {
        let mut temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let external_schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        write!(temp_file, "{external_schema}").expect("Failed to write to temp file");

        let uri = path_to_uri(temp_file.path());

        let schema = json!({
            "type": "object",
            "properties": {
                "user": { "$ref": uri }
            }
        });

        let validator = crate::validator_for(&schema).expect("Schema compilation failed");

        let valid = json!({"user": {"name": "John Doe"}});
        assert!(validator.is_valid(&valid));

        let invalid = json!({"user": {}});
        assert!(!validator.is_valid(&invalid));
    }

    #[test]
    fn test_unknown_scheme() {
        let schema = json!({
            "type": "object",
            "properties": {
                "test": { "$ref": "unknown-schema://test" }
            }
        });

        let result = crate::validator_for(&schema);

        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        assert!(error.contains("Unknown scheme"));
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        assert!(error.contains("External references are not supported on wasm32-unknown-unknown"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn create_temp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> String {
        let file_path = dir.path().join(name);
        std::fs::write(&file_path, content).unwrap();
        file_path.to_str().unwrap().to_string()
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_http_retriever_with_default_options() {
        let options = HttpOptions::new();
        let retriever = HttpRetriever::new(&options);
        assert!(retriever.is_ok());
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_http_retriever_nonexistent_cert() {
        let options = HttpOptions::new().add_root_certificate("/nonexistent/cert.pem");
        let result = HttpRetriever::new(&options);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, HttpRetrieverError::CertificateRead { .. }));
        assert!(err.to_string().contains("/nonexistent/cert.pem"));
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_http_retriever_error_source() {
        use std::error::Error;

        let options = HttpOptions::new().add_root_certificate("/nonexistent/cert.pem");
        let err = HttpRetriever::new(&options).unwrap_err();
        assert!(err.source().is_some());
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_http_retriever_with_valid_certificate() {
        // Test certificate file generated with:
        // openssl req -x509 -newkey rsa:2048 -keyout /dev/null -out cert.pem -days 3650 -nodes -subj "/CN=test"
        let cert_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test_cert.pem");
        let options = HttpOptions::new().add_root_certificate(&cert_path);
        let retriever = HttpRetriever::new(&options);
        assert!(retriever.is_ok(), "Failed: {:?}", retriever.err());
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_with_http_options_default() {
        let http_options = HttpOptions::new();
        let schema = json!({"type": "string"});
        let result = crate::options().with_http_options(&http_options);
        assert!(result.is_ok());
        let validator = result.unwrap().build(&schema);
        assert!(validator.is_ok());
        assert!(validator.unwrap().is_valid(&json!("test")));
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_with_http_options_with_timeouts() {
        use std::time::Duration;

        let http_options = HttpOptions::new()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30));
        let schema = json!({"type": "integer"});
        let result = crate::options().with_http_options(&http_options);
        assert!(result.is_ok());
        let validator = result.unwrap().build(&schema).unwrap();
        assert!(validator.is_valid(&json!(42)));
        assert!(!validator.is_valid(&json!("not an integer")));
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_with_http_options_invalid_cert() {
        let http_options = HttpOptions::new().add_root_certificate("/nonexistent/cert.pem");
        let result = crate::options().with_http_options(&http_options);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("/nonexistent/cert.pem"));
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_with_http_options_danger_accept_invalid_certs() {
        let http_options = HttpOptions::new().danger_accept_invalid_certs(true);
        let schema = json!({"type": "boolean"});
        let result = crate::options().with_http_options(&http_options);
        assert!(result.is_ok());
        let validator = result.unwrap().build(&schema).unwrap();
        assert!(validator.is_valid(&json!(true)));
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_http_retriever_retrieve_trait() {
        use referencing::Retrieve;
        use std::time::Duration;

        let options = HttpOptions::new().timeout(Duration::from_secs(30));
        let retriever = HttpRetriever::new(&options).unwrap();
        let uri =
            referencing::uri::from_str("https://json-schema.org/draft/2020-12/schema").unwrap();
        let result = retriever.retrieve(&uri);
        assert!(result.is_ok());
        let schema = result.unwrap();
        // The meta-schema should be an object with $schema and $id
        assert!(schema.is_object());
        assert!(schema.get("$schema").is_some());
    }

    #[test]
    #[cfg(all(
        feature = "resolve-http",
        feature = "resolve-file",
        not(target_arch = "wasm32")
    ))]
    fn test_http_retriever_file_scheme() {
        use referencing::Retrieve;

        let dir = tempfile::tempdir().unwrap();
        let schema_content = r#"{"type": "string"}"#;
        let schema_path = dir.path().join("schema.json");
        std::fs::write(&schema_path, schema_content).unwrap();

        let options = HttpOptions::new();
        let retriever = HttpRetriever::new(&options).unwrap();
        let uri = referencing::uri::from_str(&path_to_uri(&schema_path)).unwrap();
        let result = retriever.retrieve(&uri);
        assert!(result.is_ok());
        let schema = result.unwrap();
        assert_eq!(schema, json!({"type": "string"}));
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_http_retriever_unknown_scheme() {
        use referencing::Retrieve;

        let options = HttpOptions::new();
        let retriever = HttpRetriever::new(&options).unwrap();
        let uri = referencing::uri::from_str("ftp://example.com/schema.json").unwrap();
        let result = retriever.retrieve(&uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown scheme"));
    }

    #[test]
    #[cfg(all(feature = "resolve-http", not(target_arch = "wasm32")))]
    fn test_http_retriever_error_invalid_certificate() {
        use std::io::Write;

        let mut temp = tempfile::NamedTempFile::new().unwrap();
        temp.write_all(b"-----BEGIN CERTIFICATE-----\ninvalid\n-----END CERTIFICATE-----")
            .unwrap();
        temp.flush().unwrap();

        let options = HttpOptions::new().add_root_certificate(temp.path());
        let result = HttpRetriever::new(&options);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, HttpRetrieverError::ClientBuild(_)));
        assert!(err.to_string().contains("Failed to build HTTP client"));
    }

    #[tokio::test]
    #[cfg(all(
        feature = "resolve-http",
        feature = "resolve-async",
        not(target_arch = "wasm32")
    ))]
    async fn test_async_http_retriever_retrieve_trait() {
        use referencing::AsyncRetrieve;
        use serde_json::Value;
        use std::time::Duration;

        let options = HttpOptions::new().timeout(Duration::from_secs(30));
        let retriever = AsyncHttpRetriever::new(&options).unwrap();
        let uri =
            referencing::uri::from_str("https://json-schema.org/draft/2020-12/schema").unwrap();
        let result: Result<Value, _> = retriever.retrieve(&uri).await;
        assert!(result.is_ok());
        let schema = result.unwrap();
        // The meta-schema should be an object with $schema and $id
        assert!(schema.is_object());
        assert!(schema.get("$schema").is_some());
    }

    #[tokio::test]
    #[cfg(all(
        feature = "resolve-http",
        feature = "resolve-async",
        feature = "resolve-file",
        not(target_arch = "wasm32")
    ))]
    async fn test_async_http_retriever_file_scheme() {
        use referencing::AsyncRetrieve;
        use serde_json::Value;

        let dir = tempfile::tempdir().unwrap();
        let schema_content = r#"{"type": "integer"}"#;
        let schema_path = dir.path().join("schema.json");
        std::fs::write(&schema_path, schema_content).unwrap();

        let options = HttpOptions::new();
        let retriever = AsyncHttpRetriever::new(&options).unwrap();
        let uri = referencing::uri::from_str(&path_to_uri(&schema_path)).unwrap();
        let result: Result<Value, _> = retriever.retrieve(&uri).await;
        assert!(result.is_ok());
        let schema = result.unwrap();
        assert_eq!(schema, json!({"type": "integer"}));
    }

    #[tokio::test]
    #[cfg(all(
        feature = "resolve-http",
        feature = "resolve-async",
        not(target_arch = "wasm32")
    ))]
    async fn test_async_http_retriever_unknown_scheme() {
        use referencing::AsyncRetrieve;
        use serde_json::Value;

        let options = HttpOptions::new();
        let retriever = AsyncHttpRetriever::new(&options).unwrap();
        let uri = referencing::uri::from_str("ftp://example.com/schema.json").unwrap();
        let result: Result<Value, _> = retriever.retrieve(&uri).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown scheme"));
    }

    #[test]
    #[cfg(all(not(target_arch = "wasm32"), feature = "resolve-file"))]
    fn test_with_base_uri_resolution() {
        let dir = tempfile::tempdir().unwrap();

        let b_schema = r#"
        {
            "type": "object",
            "properties": {
                "age": { "type": "number" }
            },
            "required": ["age"]
        }
        "#;
        let _b_path = create_temp_file(&dir, "b.json", b_schema);

        let a_schema = r#"
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "./b.json",
            "type": "object"
        }
        "#;
        let a_path = create_temp_file(&dir, "a.json", a_schema);

        let valid_instance = serde_json::json!({ "age": 30 });

        let schema_str = std::fs::read_to_string(&a_path).unwrap();
        let schema_json: serde_json::Value = serde_json::from_str(&schema_str).unwrap();

        let base_uri = path_to_uri(dir.path());
        let validator = crate::options()
            .with_base_uri(format!("{base_uri}/"))
            .build(&schema_json)
            .expect("Schema compilation failed");

        assert!(validator.is_valid(&valid_instance));

        let invalid_instance = serde_json::json!({ "age": "thirty" });
        assert!(!validator.is_valid(&invalid_instance));
    }
}

#[cfg(all(test, feature = "resolve-async", not(target_arch = "wasm32")))]
mod async_tests {
    use super::*;
    use crate::Registry;
    use serde_json::json;
    use std::io::Write;

    #[tokio::test]
    #[cfg(feature = "resolve-file")]
    async fn test_async_retrieve_from_file() {
        let mut temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let external_schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        write!(temp_file, "{external_schema}").expect("Failed to write to temp file");

        let uri = path_to_uri(temp_file.path());

        let schema = json!({
            "type": "object",
            "properties": {
                "user": { "$ref": uri }
            }
        });

        // Create registry with default async retriever
        let registry = Registry::options()
            .async_retriever(DefaultRetriever)
            .build([(
                "http://example.com/schema",
                crate::Draft::Draft202012.create_resource(schema.clone()),
            )])
            .await
            .expect("Registry creation failed");

        let validator = crate::options()
            .with_registry(registry)
            .build(&schema)
            .expect("Invalid schema");

        let valid = json!({"user": {"name": "John Doe"}});
        assert!(validator.is_valid(&valid));

        let invalid = json!({"user": {}});
        assert!(!validator.is_valid(&invalid));
    }

    #[tokio::test]
    async fn test_async_unknown_scheme() {
        let schema = json!({
            "type": "object",
            "properties": {
                "test": { "$ref": "unknown-schema://test" }
            }
        });

        let result = Registry::options()
            .async_retriever(DefaultRetriever)
            .build([(
                "http://example.com/schema",
                crate::Draft::Draft202012.create_resource(schema),
            )])
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("Unknown scheme"));
    }

    #[tokio::test]
    #[cfg(feature = "resolve-file")]
    async fn test_async_concurrent_retrievals() {
        let mut temp_files = vec![];
        let mut uris = vec![];

        // Create multiple temp files with different schemas
        for i in 0..3 {
            let mut temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
            let schema = json!({
                "type": "object",
                "properties": {
                    "field": { "type": "string", "minLength": i }
                }
            });
            write!(temp_file, "{schema}").expect("Failed to write to temp file");
            uris.push(path_to_uri(temp_file.path()));
            temp_files.push(temp_file);
        }

        // Create a schema that references all temp files
        let schema = json!({
            "type": "object",
            "properties": {
                "obj1": { "$ref": uris[0] },
                "obj2": { "$ref": uris[1] },
                "obj3": { "$ref": uris[2] }
            }
        });

        let registry = Registry::options()
            .async_retriever(DefaultRetriever)
            .build([(
                "http://example.com/schema",
                crate::Draft::Draft202012.create_resource(schema.clone()),
            )])
            .await
            .expect("Registry creation failed");

        let validator = crate::options()
            .with_registry(registry)
            .build(&schema)
            .expect("Invalid schema");

        let valid = json!({
            "obj1": { "field": "" },      // minLength: 0
            "obj2": { "field": "a" },     // minLength: 1
            "obj3": { "field": "ab" }     // minLength: 2
        });
        assert!(validator.is_valid(&valid));

        // Test invalid data
        let invalid = json!({
            "obj1": { "field": "" },
            "obj2": { "field": "" },      // should be at least 1 char
            "obj3": { "field": "a" }      // should be at least 2 chars
        });
        assert!(!validator.is_valid(&invalid));
    }
}
