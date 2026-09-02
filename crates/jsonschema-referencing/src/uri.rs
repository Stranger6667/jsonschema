//! URI handling utilities for JSON Schema references.
use fluent_uri::{
    pct_enc::{encoder::Fragment, EStr, EString},
    Uri, UriRef,
};
use std::sync::LazyLock;

use crate::Error;
pub use fluent_uri::pct_enc::encoder::Path;

/// Resolves the URI reference against the given base URI and returns the target URI.
///
/// # Errors
///
/// Returns an error if `uri` is not a valid URI reference or cannot be resolved against `base`.
pub fn resolve_against(base: &Uri<&str>, uri: &str) -> Result<Uri<String>, Error> {
    if uri.starts_with('#') && base.as_str().ends_with(uri) {
        return Ok(base.to_owned());
    }
    // RFC 3986, 5.2.1: the base URI's fragment is undefined and takes no part in resolution.
    // Drafts 4-7 allow `$id` to carry one, so drop it rather than reject the base.
    let without_fragment;
    let base = if base.has_fragment() {
        without_fragment = base.strip_fragment();
        &without_fragment
    } else {
        base
    };
    Ok(UriRef::parse(uri)
        .map_err(|error| Error::uri_reference_parsing_error(uri, error))?
        .resolve_against(base)
        .map_err(|error| Error::uri_resolving_error(uri, *base, error))?
        .normalize())
}

/// Parses a URI reference from a string into a [`crate::Uri`].
///
/// # Errors
///
/// Returns an error if the input string does not conform to URI-reference from RFC 3986.
pub fn from_str(uri: &str) -> Result<Uri<String>, Error> {
    let uriref = UriRef::parse(uri)
        .map_err(|error| Error::uri_reference_parsing_error(uri, error))?
        .normalize();
    if uriref.has_scheme() {
        Ok(Uri::try_from(uriref.as_str())
            .map_err(|error| Error::uri_parsing_error(uriref.as_str(), error))?
            .into())
    } else {
        Ok(uriref
            .resolve_against(&DEFAULT_ROOT_URI.borrow())
            .map_err(|error| Error::uri_resolving_error(uri, DEFAULT_ROOT_URI.borrow(), error))?)
    }
}

pub(crate) static DEFAULT_ROOT_URI: LazyLock<Uri<String>> =
    LazyLock::new(|| Uri::parse("json-schema:///".to_string()).expect("Invalid URI"));

pub type EncodedString = EStr<Fragment>;
/// Reusable buffer for building a percent-encoded fragment.
pub type EncodedBuffer = EString<Fragment>;
