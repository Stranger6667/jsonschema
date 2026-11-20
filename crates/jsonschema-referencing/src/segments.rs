use std::borrow::Cow;

/// JSON Pointer segments walked since the last subresource boundary.
pub(crate) struct Segments<'a>(Vec<Segment<'a>>);

impl<'a> Segments<'a> {
    pub(crate) fn new() -> Self {
        Self(Vec::new())
    }

    pub(crate) fn push(&mut self, segment: impl Into<Segment<'a>>) {
        self.0.push(segment.into());
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Segment<'a>> {
        self.0.iter()
    }
}

/// Represents a single segment in a JSON pointer.
#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum Segment<'a> {
    /// A string key for object properties.
    Key(Cow<'a, str>),
    /// A numeric index for array elements.
    Index(usize),
}

impl<'a> From<Cow<'a, str>> for Segment<'a> {
    fn from(value: Cow<'a, str>) -> Self {
        Segment::Key(value)
    }
}

impl From<usize> for Segment<'_> {
    fn from(value: usize) -> Self {
        Segment::Index(value)
    }
}
