use core::fmt;

use fluent_uri::Uri;
use serde_json::Value;

/// Trait for retrieving resources from external sources.
///
/// Implementors of this trait can be used to fetch resources that are not
/// initially present in a [`crate::Registry`].
pub trait Retrieve: Send + Sync {
    /// Attempt to retrieve a resource from the given URI.
    ///
    /// # Arguments
    ///
    /// * `uri` - The URI of the resource to retrieve.
    ///
    /// # Errors
    ///
    /// If the resource couldn't be retrieved or an error occurred.
    fn retrieve(&self, uri: &Uri<&str>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;
}

/// A retriever that always fails, used as a default when external resource fetching is not needed.
#[derive(Debug, Clone)]
struct DefaultRetrieverError;

impl fmt::Display for DefaultRetrieverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Default retriever does not fetch resources")
    }
}

impl std::error::Error for DefaultRetrieverError {}

#[derive(Debug, PartialEq, Eq)]
pub struct DefaultRetriever;

impl Retrieve for DefaultRetriever {
    fn retrieve(&self, _: &Uri<&str>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(Box::new(DefaultRetrieverError))
    }
}
