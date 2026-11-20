use std::borrow::Cow;

use serde_json::Value;

use crate::{Draft, Error, Resolved, Resolver, Segments};

/// An owned document with a concrete interpretation under a JSON Schema specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    contents: Value,
    draft: Draft,
}

impl Resource {
    #[inline]
    pub(crate) fn new(contents: Value, draft: Draft) -> Self {
        Self { contents, draft }
    }
    #[inline]
    pub(crate) fn into_inner(self) -> (Draft, Value) {
        (self.draft, self.contents)
    }
    /// Resource contents.
    #[must_use]
    #[inline]
    pub fn contents(&self) -> &Value {
        &self.contents
    }
    /// JSON Schema draft under which this contents is interpreted.
    #[must_use]
    #[inline]
    pub fn draft(&self) -> Draft {
        self.draft
    }
    /// Create a resource whose draft comes from `$schema`, Draft 2020-12 without one.
    ///
    /// Unknown `$schema` values are treated as `Draft::Unknown`.
    #[must_use]
    #[inline]
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
    #[inline]
    pub fn new(contents: &'a Value, draft: Draft) -> Self {
        Self { contents, draft }
    }
    #[must_use]
    #[inline]
    pub fn contents(&self) -> &'a Value {
        self.contents
    }
    #[must_use]
    #[inline]
    pub fn draft(&self) -> Draft {
        self.draft
    }

    /// Create a resource reference whose draft comes from `$schema`, Draft 2020-12 without one.
    ///
    /// Unknown `$schema` values are treated as `Draft::Unknown`.
    #[must_use]
    #[inline]
    pub fn from_contents(contents: &'a Value) -> Self {
        let draft = Draft::default().detect(contents);
        Self::new(contents, draft)
    }

    #[must_use]
    #[inline]
    pub fn id(&self) -> Option<&str> {
        self.draft
            .id_of(self.contents)
            .map(|id| id.trim_end_matches('#'))
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
        // The draft `contents` is in: a `$schema` on the way switches it, as it does when the
        // schema is reached from the root.
        let mut draft = self.draft;
        let mut segments = Segments::new();
        let original_pointer = pointer;
        let pointer = &pointer[1..];
        let pointer = if pointer.contains('%') {
            percent_encoding::percent_decode_str(pointer)
                .decode_utf8()
                .map_err(|err| Error::invalid_percent_encoding(original_pointer, err))?
        } else {
            Cow::Borrowed(pointer)
        };
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
            if draft.is_subschema(&segments, contents) {
                let own = draft.detect(contents);
                let new_resolver = resolver.in_subresource(ResourceRef::new(contents, own))?;
                // Keyword positions count from the innermost resource or `$schema`, in its draft.
                if new_resolver != resolver || own != draft {
                    segments = Segments::new();
                }
                resolver = new_resolver;
                draft = own;
            }
        }
        Ok(Resolved::new(contents, resolver, draft))
    }
}

/// Unescape JSON Pointer path segments by converting `~1` to `/` and `~0` to `~`.
#[must_use]
pub fn unescape_segment(mut segment: &str) -> Cow<'_, str> {
    // One `find('~')` per escape; a segment without `~` is borrowed unchanged.
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
        Registry::new()
            .add("http://example.com", schema)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources")
    }

    #[test]
    fn test_empty_ref() {
        let schema = Draft::Draft202012.create_resource(json!({
            "type": "object",
            "properties": {
                "foo": { "type": "string" }
            }
        }));
        let registry = Registry::new()
            .add("http://example.com", &schema)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let resolved = resolver.lookup("#").expect("Lookup failed");
        assert_eq!(resolved.contents(), schema.contents());
    }

    #[test]
    fn test_percent_encoded_non_utf8() {
        let registry = create_test_registry();
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

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
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

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
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#/properties/bar/items/2");
        assert_eq!(
            result.expect_err("Should fail").to_string(),
            "Pointer '/properties/bar/items/2' does not exist"
        );
    }

    // A pointer lands in the draft and the resource that a `$schema` and an id on its way name.
    #[test_case("#/properties/a", Draft::Draft202012, "http://example.com" ; "root draft")]
    #[test_case("#/$defs/x", Draft::Draft4, "http://example.com" ; "subschema")]
    #[test_case("#/$defs/x/allOf/0", Draft::Draft4, "http://example.com" ; "below a subschema")]
    #[test_case("#/$defs/y", Draft::Draft4, "http://example.com/y" ; "resource")]
    #[test_case("#/$defs/x/definitions/z", Draft::Draft4, "http://example.com/z" ; "resource below a subschema")]
    fn test_pointer_through_nested_schema(reference: &str, draft: Draft, base_uri: &str) {
        let schema = Draft::Draft202012.create_resource(json!({
            "properties": {"a": {}},
            "$defs": {
                "x": {
                    "$schema": "http://json-schema.org/draft-04/schema#",
                    "allOf": [{}],
                    "definitions": {"z": {"id": "http://example.com/z"}}
                },
                "y": {"$schema": "http://json-schema.org/draft-04/schema#", "id": "http://example.com/y"}
            }
        }));
        let registry = Registry::new()
            .add("http://example.com", schema)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let (_, resolver, resolved_draft) = resolver
            .lookup(reference)
            .expect("Lookup failed")
            .into_inner();
        assert_eq!(
            (resolved_draft, resolver.base_uri().as_str()),
            (draft, base_uri)
        );
    }

    #[test]
    fn test_unknown_property() {
        let registry = create_test_registry();
        let resolver = registry
            .resolver(crate::uri::from_str("http://example.com").expect("Invalid base URI"));

        let result = resolver.lookup("#/properties/baz");
        assert_eq!(
            result.expect_err("Should fail").to_string(),
            "Pointer '/properties/baz' does not exist"
        );
    }
}
