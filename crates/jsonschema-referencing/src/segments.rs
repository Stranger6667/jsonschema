use core::slice;
use std::borrow::Cow;

/// Represents a sequence of segments in a JSON pointer.
///
/// Used to track the path during JSON pointer resolution.
/// In most cases, pointers are short, hence it is an enum.
#[derive(Debug)]
pub(crate) enum Segments<'a> {
    Empty,
    One(Segment<'a>),
    Two(Segment<'a>, Segment<'a>),
    Long(Vec<Segment<'a>>),
}

impl<'a> Segments<'a> {
    #[inline]
    pub(crate) fn new() -> Self {
        Segments::Empty
    }

    /// Adds a new segment to the sequence.
    #[inline]
    pub(crate) fn push(&mut self, segment: impl Into<Segment<'a>>) {
        *self = match std::mem::replace(self, Segments::Empty) {
            Segments::Empty => Segments::One(segment.into()),
            Segments::One(s1) => Segments::Two(s1, segment.into()),
            Segments::Two(s1, s2) => Segments::Long(vec![s1, s2, segment.into()]),
            Segments::Long(mut vec) => {
                vec.push(segment.into());
                Segments::Long(vec)
            }
        };
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        *self = Segments::Empty;
    }

    /// Returns an iterator over the segments.
    #[inline]
    pub(crate) fn iter(&'a self) -> SegmentIter<'a> {
        match self {
            Segments::Empty => SegmentIter::Empty,
            Segments::One(s1) => SegmentIter::One(s1),
            Segments::Two(s1, s2) => SegmentIter::Two(s1, s2),
            Segments::Long(vec) => SegmentIter::Long(vec.iter()),
        }
    }
}

pub(crate) enum SegmentIter<'a> {
    Empty,
    One(&'a Segment<'a>),
    Two(&'a Segment<'a>, &'a Segment<'a>),
    Long(slice::Iter<'a, Segment<'a>>),
}

impl<'a> Iterator for SegmentIter<'a> {
    type Item = &'a Segment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match std::mem::replace(self, SegmentIter::Empty) {
            SegmentIter::Empty => None,
            SegmentIter::One(s1) => Some(s1),
            SegmentIter::Two(s1, s2) => {
                *self = SegmentIter::One(s2);
                Some(s1)
            }
            SegmentIter::Long(mut iter) => {
                let next = iter.next();
                if next.is_some() {
                    *self = SegmentIter::Long(iter);
                }
                next
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            SegmentIter::Empty => (0, Some(0)),
            SegmentIter::One(_) => (1, Some(1)),
            SegmentIter::Two(_, _) => (2, Some(2)),
            SegmentIter::Long(iter) => iter.size_hint(),
        }
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
