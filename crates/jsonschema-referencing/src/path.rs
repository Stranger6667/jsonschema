use std::{borrow::Cow, sync::Arc};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JsonPointerSegment<'a> {
    Key(Cow<'a, str>),
    Index(usize),
}

impl From<usize> for JsonPointerSegment<'_> {
    fn from(value: usize) -> Self {
        Self::Index(value)
    }
}

impl<'a> From<&'a str> for JsonPointerSegment<'a> {
    fn from(value: &'a str) -> Self {
        Self::Key(Cow::Borrowed(value))
    }
}

impl<'a> From<&'a String> for JsonPointerSegment<'a> {
    fn from(value: &'a String) -> Self {
        Self::Key(Cow::Borrowed(value))
    }
}

impl<'a> From<Cow<'a, str>> for JsonPointerSegment<'a> {
    fn from(value: Cow<'a, str>) -> Self {
        Self::Key(value)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct JsonPointerNode<'a, 'b> {
    segment: JsonPointerSegment<'a>,
    parent: Option<&'b JsonPointerNode<'b, 'a>>,
}

impl Default for JsonPointerNode<'_, '_> {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonPointerNode<'_, '_> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            segment: JsonPointerSegment::Index(0),
            parent: None,
        }
    }
}

impl<'a, 'b> JsonPointerNode<'a, 'b> {
    #[must_use]
    pub fn push<'next>(
        &'next self,
        segment: impl Into<JsonPointerSegment<'a>>,
    ) -> JsonPointerNode<'a, 'next> {
        JsonPointerNode {
            segment: segment.into(),
            parent: Some(self),
        }
    }
    #[must_use]
    pub const fn segment(&self) -> &JsonPointerSegment<'a> {
        &self.segment
    }

    #[must_use]
    pub const fn parent(&self) -> Option<&'b JsonPointerNode<'b, 'a>> {
        self.parent
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedJsonPointer(Arc<str>);

impl OwnedJsonPointer {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'a> From<&'a JsonPointerNode<'_, 'a>> for OwnedJsonPointer {
    fn from(value: &'a JsonPointerNode<'_, 'a>) -> Self {
        let mut segments = Vec::new();
        let mut head = value;
        while let Some(parent) = head.parent {
            segments.push(&head.segment);
            head = parent;
        }

        let mut buffer = String::new();
        for segment in segments.iter().rev() {
            buffer.push('/');
            match segment {
                JsonPointerSegment::Key(key) => write_escaped_str(&mut buffer, key),
                JsonPointerSegment::Index(idx) => write_index(&mut buffer, *idx),
            }
        }
        Self(Arc::from(buffer))
    }
}

/// Escape a key into a JSON Pointer segment: `~` → `~0`, `/` → `~1`.
///
/// Appends the escaped form of `value` directly to `buffer`.
pub fn write_escaped_str(buffer: &mut String, value: &str) {
    match value.find(['~', '/']) {
        Some(mut escape_idx) => {
            let mut remaining = value;

            // Loop through the string to replace `~` and `/`
            loop {
                let (before, after) = remaining.split_at(escape_idx);
                // Copy everything before the escape char
                buffer.push_str(before);

                // Append the appropriate escape sequence
                match after.as_bytes()[0] {
                    b'~' => buffer.push_str("~0"),
                    b'/' => buffer.push_str("~1"),
                    _ => unreachable!(),
                }

                // Move past the escaped character
                remaining = &after[1..];

                // Find the next `~` or `/` to continue escaping
                if let Some(next_escape_idx) = remaining.find(['~', '/']) {
                    escape_idx = next_escape_idx;
                } else {
                    // Append any remaining part of the string
                    buffer.push_str(remaining);
                    break;
                }
            }
        }
        None => {
            // If no escape characters are found, append the segment as is
            buffer.push_str(value);
        }
    }
}

#[inline]
pub fn write_index(buffer: &mut String, idx: usize) {
    let mut itoa_buffer = itoa::Buffer::new();
    buffer.push_str(itoa_buffer.format(idx));
}

#[cfg(test)]
mod tests {
    use super::{JsonPointerNode, OwnedJsonPointer};

    #[test]
    fn test_json_pointer_node_single_index_fast_path() {
        let root = JsonPointerNode::new();
        let child = root.push(3usize);

        let pointer = OwnedJsonPointer::from(&child);

        assert_eq!(pointer.as_str(), "/3");
    }

    #[test]
    fn test_json_pointer_node_escapes_property_names() {
        let root = JsonPointerNode::new();
        let child = root.push("foo/bar~baz");

        let pointer = OwnedJsonPointer::from(&child);

        assert_eq!(pointer.as_str(), "/foo~1bar~0baz");
    }
}
