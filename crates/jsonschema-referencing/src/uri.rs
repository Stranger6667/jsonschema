//! URI handling utilities for JSON Schema references.
use fluent_uri::{
    pct_enc::{encoder::Fragment, EStr, Encoder},
    Uri, UriRef,
};
use std::{borrow::Cow, sync::LazyLock};

use crate::Error;
pub use fluent_uri::pct_enc::encoder::Path;

/// Resolves the URI reference against the given base URI and returns the target URI.
///
/// # Errors
///
/// Returns an error if base has not schema or there is a fragment.
pub fn resolve_against(base: &Uri<&str>, uri: &str) -> Result<Uri<String>, Error> {
    if uri.starts_with('#') && base.as_str().ends_with(uri) {
        return Ok(base.to_owned());
    }
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

/// Percent-encode URI-illegal characters in a fragment (e.g. `<`/`>` in a `$ref` JSON pointer), borrowing
/// unchanged when nothing needs escaping. A `%` starting a valid `%XX` triplet is treated as already encoded
/// so encoded fragments pass through unchanged (idempotent); a stray `%` is encoded to `%25`.
#[must_use]
pub fn encode_fragment(fragment: &str) -> Cow<'_, str> {
    let bytes = fragment.as_bytes();
    let starts_triplet = |index: usize| {
        bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit)
    };
    let allowed =
        |index: usize, ch: char| (ch == '%' && starts_triplet(index)) || Path::TABLE.allows(ch);
    if fragment
        .char_indices()
        .all(|(index, ch)| allowed(index, ch))
    {
        return Cow::Borrowed(fragment);
    }
    let mut buffer = String::with_capacity(fragment.len() + 8);
    for (index, ch) in fragment.char_indices() {
        if allowed(index, ch) {
            buffer.push(ch);
        } else {
            encode_to(ch.encode_utf8(&mut [0; 4]), &mut buffer);
        }
    }
    Cow::Owned(buffer)
}

// Adapted from `https://github.com/yescallop/fluent-uri-rs/blob/main/src/encoding/table.rs#L153`
pub fn encode_to(input: &str, buffer: &mut String) {
    const HEX_TABLE: [u8; 512] = {
        const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

        let mut i = 0;
        let mut table = [0; 512];
        while i < 256 {
            table[i * 2] = HEX_DIGITS[i >> 4];
            table[i * 2 + 1] = HEX_DIGITS[i & 0b1111];
            i += 1;
        }
        table
    };

    for ch in input.chars() {
        if Path::TABLE.allows(ch) {
            buffer.push(ch);
        } else {
            for x in ch.encode_utf8(&mut [0; 4]).bytes() {
                buffer.push('%');
                buffer.push(HEX_TABLE[x as usize * 2] as char);
                buffer.push(HEX_TABLE[x as usize * 2 + 1] as char);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::encode_fragment;
    use std::borrow::Cow;

    #[test]
    fn encode_fragment_behavior() {
        // Clean fragment: borrowed unchanged, no allocation.
        assert!(matches!(
            encode_fragment("/definitions/Foo"),
            Cow::Borrowed("/definitions/Foo")
        ));
        // URI-illegal `<`/`>` get encoded.
        assert_eq!(
            encode_fragment("/definitions/A<B>"),
            "/definitions/A%3CB%3E"
        );
        // Already-encoded `%XX` is preserved (idempotent), not doubled into `%2520`.
        assert!(matches!(
            encode_fragment("/definitions/foo%20bar"),
            Cow::Borrowed(_)
        ));
        assert_eq!(
            encode_fragment(&encode_fragment("/A<B>")),
            encode_fragment("/A<B>")
        );
        // A stray `%` that does not start a `%XX` triplet is itself encoded; the result is a valid URI fragment.
        assert_eq!(encode_fragment("/definitions/100%"), "/definitions/100%25");
        assert_eq!(encode_fragment("/definitions/a%zz"), "/definitions/a%25zz");
        assert_eq!(
            encode_fragment(&encode_fragment("/definitions/100%")),
            "/definitions/100%25"
        );
    }
}
