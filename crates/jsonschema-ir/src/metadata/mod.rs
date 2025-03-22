use core::fmt;
use std::{ops::Range, sync::Arc};

use string_interner::{backend::BucketBackend, symbol::SymbolU32, StringInterner};

use crate::{BlockId, JsonValue};

pub(crate) mod vocabulary;

pub type Size = usize;
type Idx = u32;

pub struct Metadata {
    pub(crate) constants: Constants,
}
impl Metadata {
    pub(crate) fn new() -> Self {
        Self {
            constants: Constants::new(),
        }
    }

    pub(crate) fn new_location(&mut self, location: Location) -> LocationId {
        let id = self.constants.locations.len() as u32;
        self.constants.locations.push(location);
        LocationId(id)
    }
}

pub struct LocationId(u32);

/// A location segment.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LocationSegment<'a> {
    /// Property name within a JSON object.
    Property(&'a str),
    /// JSON Schema keyword.
    Index(usize),
}

impl fmt::Display for LocationSegment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocationSegment::Property(property) => f.write_str(property),
            LocationSegment::Index(idx) => f.write_str(itoa::Buffer::new().format(*idx)),
        }
    }
}

impl<'a> From<&'a str> for LocationSegment<'a> {
    #[inline]
    fn from(value: &'a str) -> LocationSegment<'a> {
        LocationSegment::Property(value)
    }
}

impl<'a> From<&'a String> for LocationSegment<'a> {
    #[inline]
    fn from(value: &'a String) -> LocationSegment<'a> {
        LocationSegment::Property(value)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Location(Arc<String>);
impl Location {
    pub(crate) fn new() -> Self {
        Self(Arc::new(String::new()))
    }

    pub(crate) fn join<'a>(&self, segment: impl Into<LocationSegment<'a>>) -> Self {
        let parent = self.0.as_str();
        match segment.into() {
            LocationSegment::Property(property) => {
                let mut buffer = String::with_capacity(parent.len() + property.len() + 1);
                buffer.push_str(parent);
                buffer.push('/');
                write_escaped_str(&mut buffer, property);
                Self(Arc::new(buffer))
            }
            LocationSegment::Index(idx) => {
                let mut buffer = itoa::Buffer::new();
                let segment = buffer.format(idx);
                Self(Arc::new(format!("{parent}/{segment}")))
            }
        }
    }
}

fn write_escaped_str(buffer: &mut String, value: &str) {
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
    };
}

pub struct Locations {
    items: Vec<Location>,
}

pub(crate) struct Constants {
    strings: StringInterner<BucketBackend>,
    values: Vec<JsonValue>,
    numbers: Vec<()>,
    locations: Vec<Location>,
    /// All `required` values merged into a single block.
    required: Vec<SymbolU32>,
    definitions: Vec<(SymbolU32, BlockId)>,
    dependent_schemas: Vec<(SymbolU32, BlockId)>,
    dependent_required: Vec<(SymbolU32, Range<Idx>)>,
    properties: Vec<(SymbolU32, BlockId)>,
    pattern_properties: Vec<(SymbolU32, BlockId)>,
}
impl Constants {
    fn new() -> Self {
        Self {
            strings: StringInterner::new(),
            values: Vec::new(),
            numbers: Vec::new(),
            locations: Vec::new(),
            required: Vec::new(),
            definitions: Vec::new(),
            dependent_schemas: Vec::new(),
            dependent_required: Vec::new(),
            properties: Vec::new(),
            pattern_properties: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ConstantId(Idx);

#[cfg(feature = "arbitrary_precision")]
mod number {
    use crate::value::Number;

    use super::Idx;

    pub(crate) enum NumberId {
        Inline(Number),
        Heap(Idx),
    }
}

#[cfg(not(feature = "arbitrary_precision"))]
mod number {
    use crate::value::Number;

    pub(crate) struct NumberId(Number);

    impl From<Number> for NumberId {
        fn from(value: Number) -> Self {
            NumberId(value)
        }
    }
}

pub(crate) use number::*;

/// Identifies a value range where enum values are stored.
#[derive(Debug)]
pub(crate) struct EnumId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct RequiredId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct DependentSchemasId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct DependentRequiredId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct FormatId(SymbolU32);

#[derive(Debug)]
pub(crate) struct PropertiesId(Range<Idx>);

#[derive(Debug)]
pub(crate) struct PatternPropertiesId(Range<Idx>);
