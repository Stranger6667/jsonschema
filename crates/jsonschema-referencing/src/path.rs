use std::borrow::Cow;

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

/// Escape a key into a JSON Pointer segment: `~` -> `~0`, `/` -> `~1`.
///
/// Appends the escaped form of `value` directly to `buffer`.
pub fn write_escaped_str(buffer: &mut String, value: &str) {
    let bytes = value.as_bytes();
    // `str::find(['~', '/'])` builds a UTF-8 char searcher; `memchr2` is a SIMD byte scan.
    // Both needles are ASCII, so every index it returns is a char boundary.
    let Some(mut escape_idx) = memchr::memchr2(b'~', b'/', bytes) else {
        // If no escape characters are found, append the segment as is
        buffer.push_str(value);
        return;
    };

    let mut start = 0;
    loop {
        // Copy everything before the escape char
        buffer.push_str(&value[start..escape_idx]);

        // Append the appropriate escape sequence
        buffer.push_str(if bytes[escape_idx] == b'~' {
            "~0"
        } else {
            "~1"
        });

        // Move past the escaped character
        start = escape_idx + 1;

        // Find the next `~` or `/` to continue escaping
        if let Some(next) = memchr::memchr2(b'~', b'/', &bytes[start..]) {
            escape_idx = start + next;
        } else {
            // Append any remaining part of the string
            buffer.push_str(&value[start..]);
            break;
        }
    }
}

#[inline]
pub fn write_index(buffer: &mut String, idx: usize) {
    let mut itoa_buffer = itoa::Buffer::new();
    buffer.push_str(itoa_buffer.format(idx));
}
