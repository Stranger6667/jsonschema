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
    let mut remaining = value;
    while let Some(index) = first_escape(remaining.as_bytes()) {
        buffer.push_str(&remaining[..index]);
        buffer.push_str(if remaining.as_bytes()[index] == b'~' {
            "~0"
        } else {
            "~1"
        });
        remaining = &remaining[index + 1..];
    }
    buffer.push_str(remaining);
}

const ONES: u64 = 0x0101_0101_0101_0101;
const HIGHS: u64 = 0x8080_8080_8080_8080;

/// High bit set in each byte of `word` that is `~` or `/`; exact for the lowest such byte.
#[inline]
fn escape_mask(word: u64) -> u64 {
    let tilde = word ^ (ONES * u64::from(b'~'));
    let slash = word ^ (ONES * u64::from(b'/'));
    ((tilde.wrapping_sub(ONES) & !tilde) | (slash.wrapping_sub(ONES) & !slash)) & HIGHS
}

/// Position of the first `~` or `/`, eight bytes per step.
#[inline]
fn first_escape(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0;
    let mut rest = bytes;
    while let Some((word, tail)) = rest.split_first_chunk::<8>() {
        let mask = escape_mask(u64::from_le_bytes(*word));
        if mask != 0 {
            return Some(offset + (mask.trailing_zeros() / 8) as usize);
        }
        offset += 8;
        rest = tail;
    }
    rest.iter()
        .position(|&byte| byte == b'~' || byte == b'/')
        .map(|index| offset + index)
}

#[inline]
pub fn write_index(buffer: &mut String, idx: usize) {
    let mut itoa_buffer = itoa::Buffer::new();
    buffer.push_str(itoa_buffer.format(idx));
}

#[cfg(test)]
mod tests {
    use super::write_escaped_str;
    use test_case::test_case;

    #[test_case("", ""; "empty")]
    #[test_case("abc", "abc"; "short plain")]
    #[test_case("abcdefgh", "abcdefgh"; "one word plain")]
    #[test_case("abcdefghijklmnopq", "abcdefghijklmnopq"; "two words and a tail plain")]
    #[test_case("~", "~0"; "tilde alone")]
    #[test_case("/", "~1"; "slash alone")]
    #[test_case("a/b", "a~1b"; "slash inside")]
    #[test_case("abcdefg~", "abcdefg~0"; "tilde in the last byte of a word")]
    #[test_case("abcdefgh/", "abcdefgh~1"; "slash in the tail")]
    #[test_case("abcdefghijklmno~pqrstuvw", "abcdefghijklmno~0pqrstuvw"; "tilde in the second word")]
    #[test_case("~/~", "~0~1~0"; "every byte escaped")]
    #[test_case("a/b/c/d/e/f/g/h/i/j", "a~1b~1c~1d~1e~1f~1g~1h~1i~1j"; "escape in every word")]
    #[test_case("/v1/accounts/{account-key}", "~1v1~1accounts~1{account-key}"; "path key")]
    #[test_case("abcdefgh/ijklmnop~qrstuvwx", "abcdefgh~1ijklmnop~0qrstuvwx"; "escapes at word starts")]
    fn escapes(value: &str, expected: &str) {
        let mut buffer = String::from("/");
        write_escaped_str(&mut buffer, value);
        assert_eq!(buffer, format!("/{expected}"));
    }
}
