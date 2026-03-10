use std::{borrow::Cow, fmt::Write};

use serde_json::Value;

use crate::{Draft, Error, Resolved, Resolver, Segments};

/// A segment in a JSON Pointer path, stored lazily to avoid string allocation during traversal.
pub(crate) enum PathSegment<'a> {
    Key(&'a str),
    Index(usize),
}

/// A lazy JSON Pointer path that avoids string building during schema traversal.
/// The path is only materialized to a `String` when actually needed (e.g., for skeleton entries).
pub(crate) struct PathStack<'a> {
    /// Owned initial path from the queue entry (empty string for root resources).
    base: String,
    /// Dynamically accumulated segments as the traversal descends.
    segments: Vec<PathSegment<'a>>,
}

impl<'a> PathStack<'a> {
    pub(crate) fn from_base(base: String) -> Self {
        Self {
            base,
            segments: Vec::new(),
        }
    }

    /// Push a key segment. Returns a checkpoint to restore with `truncate`.
    #[inline]
    pub(crate) fn push_key(&mut self, key: &'a str) -> usize {
        let checkpoint = self.segments.len();
        self.segments.push(PathSegment::Key(key));
        checkpoint
    }

    /// Push a numeric index segment. Returns a checkpoint to restore with `truncate`.
    #[inline]
    pub(crate) fn push_index(&mut self, idx: usize) -> usize {
        let checkpoint = self.segments.len();
        self.segments.push(PathSegment::Index(idx));
        checkpoint
    }

    /// Restore the stack to the given checkpoint (removing segments added after it).
    #[inline]
    pub(crate) fn truncate(&mut self, checkpoint: usize) {
        self.segments.truncate(checkpoint);
    }

    /// Materialize the full JSON Pointer path as an owned `String`.
    /// Only called when a skeleton entry is actually needed.
    pub(crate) fn to_pointer(&self) -> String {
        if self.segments.is_empty() {
            return self.base.clone();
        }
        let mut s = self.base.clone();
        for seg in &self.segments {
            match seg {
                PathSegment::Key(k) => {
                    s.push('/');
                    write_escaped_str(&mut s, k);
                }
                PathSegment::Index(i) => {
                    write!(s, "/{i}").unwrap();
                }
            }
        }
        s
    }
}

pub(crate) trait JsonSchemaResource {
    fn contents(&self) -> &Value;
    fn draft(&self) -> Draft;
    fn id(&self) -> Option<&str> {
        self.draft()
            .id_of(self.contents())
            .map(|id| id.trim_end_matches('#'))
    }
}

/// An owned document with a concrete interpretation under a JSON Schema specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    contents: Value,
    draft: Draft,
}

impl Resource {
    pub(crate) fn new(contents: Value, draft: Draft) -> Self {
        Self { contents, draft }
    }
    pub(crate) fn into_inner(self) -> (Draft, Value) {
        (self.draft, self.contents)
    }
    /// Resource contents.
    #[must_use]
    pub fn contents(&self) -> &Value {
        &self.contents
    }
    /// JSON Schema draft under which this contents is interpreted.
    #[must_use]
    pub fn draft(&self) -> Draft {
        self.draft
    }
    /// Create a resource with automatically detecting specification which applies to the contents.
    ///
    /// Unknown `$schema` values are treated as `Draft::Unknown`.
    #[must_use]
    pub fn from_contents(contents: Value) -> Resource {
        Draft::default().detect(&contents).create_resource(contents)
    }
}

/// A borrowed document with a concrete interpretation under a JSON Schema specification.
#[derive(Debug, Clone, Copy)]
pub struct ResourceRef<'a> {
    contents: &'a Value,
    draft: Draft,
}

impl<'a> ResourceRef<'a> {
    #[must_use]
    pub fn new(contents: &'a Value, draft: Draft) -> Self {
        Self { contents, draft }
    }
    #[must_use]
    pub fn contents(&self) -> &'a Value {
        self.contents
    }
    #[must_use]
    pub fn draft(&self) -> Draft {
        self.draft
    }

    /// Create a resource-ref with automatically detecting specification which applies to the contents.
    ///
    /// Unknown `$schema` values are treated as `Draft::Unknown`.
    #[must_use]
    pub fn from_contents(contents: &'a Value) -> Self {
        let draft = Draft::default().detect(contents);
        Self::new(contents, draft)
    }

    #[must_use]
    pub fn id(&self) -> Option<&str> {
        JsonSchemaResource::id(self)
    }
}

impl JsonSchemaResource for ResourceRef<'_> {
    fn contents(&self) -> &Value {
        self.contents
    }

    fn draft(&self) -> Draft {
        self.draft
    }
}

impl<'r> ResourceRef<'r> {
    pub(crate) fn pointer(
        self,
        pointer: &str,
        mut resolver: Resolver<'r>,
    ) -> Result<Resolved<'r>, Error> {
        // INVARIANT: Pointer always starts with `/`
        let mut contents = self.contents;
        let mut segments = Segments::new();
        let original_pointer = pointer;
        let pointer = percent_encoding::percent_decode_str(&pointer[1..])
            .decode_utf8()
            .map_err(|err| Error::invalid_percent_encoding(original_pointer, err))?;
        for segment in pointer.split('/') {
            if let Some(array) = contents.as_array() {
                let idx = segment
                    .parse::<usize>()
                    .map_err(|err| Error::invalid_array_index(original_pointer, segment, err))?;
                if let Some(next) = array.get(idx) {
                    contents = next;
                } else {
                    return Err(Error::pointer_to_nowhere(original_pointer));
                }
                segments.push(idx);
            } else {
                let segment = unescape_segment(segment);
                if let Some(next) = contents.get(segment.as_ref()) {
                    contents = next;
                } else {
                    return Err(Error::pointer_to_nowhere(original_pointer));
                }
                segments.push(segment);
            }
            let last = &resolver;
            let new_resolver = self.draft.maybe_in_subresource(
                &segments,
                &resolver,
                ResourceRef::new(contents, self.draft),
            )?;
            if new_resolver != *last {
                segments = Segments::new();
            }
            resolver = new_resolver;
        }
        Ok(Resolved::new(contents, resolver, self.draft))
    }
}

/// Escape a key into a JSON Pointer segment: `~` → `~0`, `/` → `~1`.
///
/// Appends the escaped form of `value` directly to `buffer`.
pub fn write_escaped_str(buffer: &mut String, value: &str) {
    match value.find(['~', '/']) {
        Some(mut escape_idx) => {
            let mut remaining = value;
            loop {
                let (before, after) = remaining.split_at(escape_idx);
                buffer.push_str(before);
                match after.as_bytes()[0] {
                    b'~' => buffer.push_str("~0"),
                    b'/' => buffer.push_str("~1"),
                    _ => unreachable!(),
                }
                remaining = &after[1..];
                if let Some(next_escape_idx) = remaining.find(['~', '/']) {
                    escape_idx = next_escape_idx;
                } else {
                    buffer.push_str(remaining);
                    break;
                }
            }
        }
        None => buffer.push_str(value),
    }
}

/// Unescape JSON Pointer path segments by converting `~1` to `/` and `~0` to `~`.
#[must_use]
pub fn unescape_segment(mut segment: &str) -> Cow<'_, str> {
    // Naively, checking for `~` and then replacing implies two passes
    // over the input buffer. First, search in the first `contains('~')` call
    // and then replacing `~1` & `~0` at once in a single pass.
    //
    // This implementation is ~3x faster than the naive one.
    //
    // **NOTE**: Heavily inspired by the implementation in `boon`:
    // `https://github.com/santhosh-tekuri/boon/blob/fb09df2db19be75c32c0970b4bdedf1655f5f943/src/util.rs#L31`
    let Some(mut tilde_idx) = segment.find('~') else {
        return Cow::Borrowed(segment);
    };

    let mut buffer = String::with_capacity(segment.len());
    loop {
        let (before, after) = segment.split_at(tilde_idx);
        buffer.push_str(before);
        segment = &after[1..];
        let next_char_size = match segment.chars().next() {
            Some('1') => {
                buffer.push('/');
                1
            }
            Some('0') => {
                buffer.push('~');
                1
            }
            Some(next) => {
                buffer.push('~');
                buffer.push(next);
                next.len_utf8()
            }
            None => {
                buffer.push('~');
                break;
            }
        };
        segment = &segment[next_char_size..];
        let Some(next_tilde_idx) = segment.find('~') else {
            buffer.push_str(segment);
            break;
        };
        tilde_idx = next_tilde_idx;
    }
    Cow::Owned(buffer)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::{Draft, Registry};

    use super::unescape_segment;
    use serde_json::json;
    use test_case::test_case;

    #[test_case("abc")]
    #[test_case("a~0b")]
    #[test_case("a~1b")]
    #[test_case("~01")]
    #[test_case("~10")]
    #[test_case("a~0~1b")]
    #[test_case("~"; "single tilde")]
    #[test_case("~~"; "double tilde")]
    #[test_case("~~~~~"; "many tildas")]
    #[test_case("~2")]
    #[test_case("a~c")]
    #[test_case("~0~1~")]
    #[test_case("")]
    #[test_case("a/d")]
    #[test_case("a~01b")]
    #[test_case("🌟~0🌠~1🌡️"; "Emojis with escapes")]
    #[test_case("~🌟"; "Tilde followed by emoji")]
    #[test_case("Café~0~1"; "Accented characters with escapes")]
    #[test_case("~é"; "Tilde followed by accented character")]
    #[test_case("αβγ"; "Greek")]
    #[test_case("~αβγ"; "Tilde followed by Greek")]
    #[test_case("∀∂∈ℝ∧∪≡∞"; "Mathematical symbols")]
    #[test_case("~∀∂∈ℝ∧∪≡∞"; "Tilde followed by mathematical symbols")]
    #[test_case("¡¢£¤¥¦§¨©"; "Special characters")]
    #[test_case("~¡¢£¤¥¦§¨©"; "Tilde followed by special characters")]
    #[test_case("\u{10FFFF}"; "Highest valid Unicode code point")]
    #[test_case("~\u{10FFFF}"; "Tilde followed by highest valid Unicode code point")]
    fn test_unescape_segment_equivalence(input: &str) {
        let unescaped = unescape_segment(input);
        let double_replaced = input.replace("~1", "/").replace("~0", "~");
        assert_eq!(unescaped, double_replaced, "Failed for: {input}");
    }

    fn create_test_registry() -> Registry<'static> {
        let schema = Draft::Draft202012.create_resource(json!({
            "type": "object",
            "properties": {
                "foo": { "type": "string" },
                "bar": { "type": "array", "items": [{"type": "number"}, {"type": "boolean"}] }
            }
        }));
        Registry::try_new("http://example.com", schema).expect("Invalid resources")
    }

    #[test]
    fn test_empty_ref() {
        let schema = Draft::Draft202012.create_resource(json!({
            "type": "object",
            "properties": {
                "foo": { "type": "string" }
            }
        }));
        let registry =
            Registry::try_new("http://example.com", schema.clone()).expect("Invalid resources");
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let resolved = resolver.lookup("#").expect("Lookup failed");
        assert_eq!(resolved.contents(), schema.contents());
    }

    #[test]
    fn test_percent_encoded_non_utf8() {
        let registry = create_test_registry();
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#/%FF");
        let error = result.expect_err("Should fail");
        assert_eq!(
            error.to_string(),
            "Invalid percent encoding in pointer '/%FF': the decoded bytes do not represent valid UTF-8"
        );
        assert!(error.source().is_some());
    }

    #[test]
    fn test_array_index_as_string() {
        let registry = create_test_registry();
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#/properties/bar/items/one");
        let error = result.expect_err("Should fail");
        assert_eq!(
            error.to_string(),
            "Failed to parse array index 'one' in pointer '/properties/bar/items/one'"
        );
        assert!(error.source().is_some());
    }

    #[test]
    fn test_array_index_out_of_bounds() {
        let registry = create_test_registry();
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#/properties/bar/items/2");
        assert_eq!(
            result.expect_err("Should fail").to_string(),
            "Pointer '/properties/bar/items/2' does not exist"
        );
    }

    #[test]
    fn test_unknown_property() {
        let registry = create_test_registry();
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#/properties/baz");
        assert_eq!(
            result.expect_err("Should fail").to_string(),
            "Pointer '/properties/baz' does not exist"
        );
    }
}
