//! Facilities for working with paths within schemas or validated instances.
use std::{borrow::Cow, fmt, sync::Arc};

use referencing::unescape_segment;

use crate::keywords::Keyword;

/// A location segment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocationSegment<'a> {
    /// Property name within a JSON object.
    Property(Cow<'a, str>),
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

/// A lazily constructed location within a JSON instance.
///
/// [`LazyLocation`] builds a path incrementally during JSON Schema validation without allocating
/// memory until required by storing each segment on the stack.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LazyLocation<'a, 'b> {
    pub(crate) segment: LocationSegment<'a>,
    pub(crate) parent: Option<&'b LazyLocation<'b, 'a>>,
}

impl Default for LazyLocation<'_, '_> {
    fn default() -> Self {
        LazyLocation::new()
    }
}

impl<'a> LazyLocation<'a, '_> {
    /// Create a root node of a JSON pointer.
    #[must_use]
    pub const fn new() -> Self {
        LazyLocation {
            // The value does not matter, it will never be used
            segment: LocationSegment::Index(0),
            parent: None,
        }
    }

    /// Push a new segment to the JSON pointer.
    #[inline]
    #[must_use]
    pub fn push(&'a self, segment: impl Into<LocationSegment<'a>>) -> Self {
        LazyLocation {
            segment: segment.into(),
            parent: Some(self),
        }
    }
}

impl<'a> From<&'a LazyLocation<'_, '_>> for Location {
    fn from(value: &'a LazyLocation<'_, '_>) -> Self {
        // Stack capacity for typical paths (≤16 segments), fallback to Vec for deep paths
        const STACK_CAPACITY: usize = 16;

        // First pass: count segments and calculate string capacity
        let mut capacity = 0;
        let mut string_capacity = 0;
        let mut head = value;

        while let Some(next) = head.parent {
            capacity += 1;
            string_capacity += match &head.segment {
                LocationSegment::Property(property) => property.len() + 1,
                LocationSegment::Index(idx) => idx.checked_ilog10().unwrap_or(0) as usize + 2,
            };
            head = next;
        }

        let mut buffer = String::with_capacity(string_capacity);

        if capacity <= STACK_CAPACITY {
            // Stack-allocated storage with references - no cloning needed
            let mut stack_segments: [Option<&LocationSegment<'_>>; STACK_CAPACITY] =
                [None; STACK_CAPACITY];
            let mut idx = 0;
            head = value;

            if head.parent.is_some() {
                stack_segments[idx] = Some(&head.segment);
                idx += 1;
            }

            while let Some(next) = head.parent {
                head = next;
                if head.parent.is_some() {
                    stack_segments[idx] = Some(&head.segment);
                    idx += 1;
                }
            }

            // Format in reverse order
            for segment in stack_segments[..idx].iter().rev().flatten() {
                buffer.push('/');
                match segment {
                    LocationSegment::Property(property) => {
                        write_escaped_str(&mut buffer, property);
                    }
                    LocationSegment::Index(idx) => {
                        let mut itoa_buffer = itoa::Buffer::new();
                        buffer.push_str(itoa_buffer.format(*idx));
                    }
                }
            }
        } else {
            // Heap-allocated fallback for deep paths (>16 segments)
            let mut segments: Vec<&LocationSegment<'_>> = Vec::with_capacity(capacity);
            head = value;

            if head.parent.is_some() {
                segments.push(&head.segment);
            }

            while let Some(next) = head.parent {
                head = next;
                if head.parent.is_some() {
                    segments.push(&head.segment);
                }
            }

            for segment in segments.iter().rev() {
                buffer.push('/');
                match segment {
                    LocationSegment::Property(property) => {
                        write_escaped_str(&mut buffer, property);
                    }
                    LocationSegment::Index(idx) => {
                        let mut itoa_buffer = itoa::Buffer::new();
                        buffer.push_str(itoa_buffer.format(*idx));
                    }
                }
            }
        }

        Location(Arc::from(buffer))
    }
}

/// A lazily constructed ref path for evaluation path tracking.
///
/// Like [`LazyLocation`], uses stack-based references - no heap allocation.
/// Each `$ref` traversal creates a new stack-local entry that borrows from
/// the parent, automatically "popping" when the function returns.
pub(crate) struct LazyRefPath<'a, 'b> {
    /// Location of the $ref keyword. `None` for root.
    pub(crate) ref_location: Option<&'a Location>,
    /// Base path to strip when computing evaluation path. `None` for root.
    pub(crate) strip_base: Option<&'a Location>,
    pub(crate) parent: Option<&'b LazyRefPath<'b, 'a>>,
    /// Cached evaluation prefix - computed lazily on first access.
    /// Uses Cell for interior mutability without synchronization overhead.
    cached_prefix: std::cell::Cell<Option<Location>>,
}

impl std::fmt::Debug for LazyRefPath<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyRefPath")
            .field("ref_location", &self.ref_location)
            .field("strip_base", &self.strip_base)
            .field("parent", &self.parent.map(|_| "..."))
            .finish()
    }
}

impl<'a> LazyRefPath<'a, '_> {
    /// Create root (empty) ref path.
    #[must_use]
    pub(crate) const fn new() -> Self {
        LazyRefPath {
            ref_location: None,
            strip_base: None,
            parent: None,
            cached_prefix: std::cell::Cell::new(None),
        }
    }

    /// Push a new $ref onto the path.
    #[inline]
    #[must_use]
    pub(crate) fn push(&'a self, ref_location: &'a Location, strip_base: &'a Location) -> Self {
        LazyRefPath {
            ref_location: Some(ref_location),
            strip_base: Some(strip_base),
            parent: Some(self),
            cached_prefix: std::cell::Cell::new(None),
        }
    }

    /// Check if this is the root (no $refs).
    #[inline]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.parent.is_none()
    }

    /// Get the cached evaluation prefix, computing it if necessary.
    /// This is O(1) after first call thanks to Cell caching.
    /// Parent prefixes are also cached, so computation is incremental.
    /// Returns a clone of the cached Location (cheap due to Arc).
    #[inline]
    pub(crate) fn get_eval_prefix(&self) -> Location {
        // Take the cached value to check it
        if let Some(prefix) = self.cached_prefix.take() {
            // Put it back and return
            self.cached_prefix.set(Some(prefix.clone()));
            prefix
        } else {
            // Compute the prefix
            let prefix = self.compute_eval_prefix_inner();
            self.cached_prefix.set(Some(prefix.clone()));
            prefix
        }
    }

    /// Compute the evaluation prefix for this node.
    /// Uses parent's cached prefix for O(1) incremental computation.
    fn compute_eval_prefix_inner(&self) -> Location {
        let Some(parent) = self.parent else {
            // Root node - empty prefix (shouldn't normally be called)
            return Location::new();
        };

        let ref_location = self.ref_location.unwrap();

        // Check if parent is root (no grandparent)
        if parent.parent.is_none() {
            // This is the first $ref - prefix is just the ref_location
            return ref_location.clone();
        }

        // Parent has its own ref - get parent's prefix and extend it
        let parent_prefix = parent.get_eval_prefix();
        let prev_strip_base = parent.strip_base.unwrap();

        if let Some(suffix) = ref_location.as_str().strip_prefix(prev_strip_base.as_str()) {
            // Append suffix directly - it's already a valid JSON pointer path
            parent_prefix.join_raw_suffix(suffix)
        } else {
            parent_prefix.clone()
        }
    }
}

impl<'a> From<&'a Keyword> for LocationSegment<'a> {
    fn from(value: &'a Keyword) -> Self {
        match value {
            Keyword::Builtin(k) => LocationSegment::Property(k.as_str().into()),
            Keyword::Custom(s) => LocationSegment::Property(Cow::Borrowed(s)),
        }
    }
}

impl<'a> From<&'a str> for LocationSegment<'a> {
    #[inline]
    fn from(value: &'a str) -> LocationSegment<'a> {
        LocationSegment::Property(Cow::Borrowed(value))
    }
}

impl<'a> From<&'a String> for LocationSegment<'a> {
    #[inline]
    fn from(value: &'a String) -> LocationSegment<'a> {
        LocationSegment::Property(Cow::Borrowed(value))
    }
}

impl<'a> From<Cow<'a, str>> for LocationSegment<'a> {
    #[inline]
    fn from(value: Cow<'a, str>) -> LocationSegment<'a> {
        LocationSegment::Property(value)
    }
}

impl From<usize> for LocationSegment<'_> {
    #[inline]
    fn from(value: usize) -> Self {
        LocationSegment::Index(value)
    }
}

/// A cheap to clone JSON pointer that represents location with a JSON value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Location(Arc<str>);

impl serde::Serialize for Location {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl Location {
    /// Create a new, empty `Location`.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::from(""))
    }

    pub(crate) fn from_escaped(escaped: &str) -> Self {
        Self(Arc::from(escaped))
    }

    /// Append a raw JSON pointer suffix (already escaped).
    /// This is more efficient than multiple `join` calls when the suffix
    /// is already a valid JSON pointer path.
    #[must_use]
    pub(crate) fn join_raw_suffix(&self, suffix: &str) -> Self {
        if suffix.is_empty() {
            return self.clone();
        }
        let parent = &self.0;
        let mut buffer = String::with_capacity(parent.len() + suffix.len());
        buffer.push_str(parent);
        buffer.push_str(suffix);
        Self(Arc::from(buffer))
    }

    #[must_use]
    pub fn join<'a>(&self, segment: impl Into<LocationSegment<'a>>) -> Self {
        let parent = &self.0;
        match segment.into() {
            LocationSegment::Property(property) => {
                let mut buffer = String::with_capacity(parent.len() + property.len() + 1);
                buffer.push_str(parent);
                buffer.push('/');
                write_escaped_str(&mut buffer, &property);
                Self(Arc::from(buffer))
            }
            LocationSegment::Index(idx) => {
                let mut itoa_buf = itoa::Buffer::new();
                let segment = itoa_buf.format(idx);
                let mut buffer = String::with_capacity(parent.len() + segment.len() + 1);
                buffer.push_str(parent);
                buffer.push('/');
                buffer.push_str(segment);
                Self(Arc::from(buffer))
            }
        }
    }
    /// Get a clone of the inner `Arc<str>` representing the location.
    #[must_use]
    pub(crate) fn as_arc(&self) -> Arc<str> {
        Arc::clone(&self.0)
    }

    /// Get a string slice representing the location.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Get a byte slice representing the location.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn iter(&self) -> std::vec::IntoIter<LocationSegment<'_>> {
        <&Self as IntoIterator>::into_iter(self)
    }
}

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

impl Default for Location {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'a> IntoIterator for &'a Location {
    type Item = LocationSegment<'a>;
    type IntoIter = std::vec::IntoIter<LocationSegment<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_str()
            .split('/')
            .filter(|p| !p.is_empty())
            .map(|p| {
                p.parse::<usize>().map_or(
                    LocationSegment::Property(unescape_segment(p)),
                    LocationSegment::Index,
                )
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl<'a> FromIterator<LocationSegment<'a>> for Location {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = LocationSegment<'a>>,
    {
        fn inner<'a, 'b, 'c, I>(path_iter: &mut I, location: &'b LazyLocation<'b, 'a>) -> Location
        where
            I: Iterator<Item = LocationSegment<'c>>,
        {
            let Some(path) = path_iter.next() else {
                return location.into();
            };
            let location = location.push(path);
            inner(path_iter, &location)
        }

        let loc = LazyLocation::default();
        inner(&mut iter.into_iter(), &loc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use test_case::test_case;

    #[test]
    fn test_location_default() {
        let loc = Location::default();
        assert_eq!(loc.as_str(), "");
    }

    #[test]
    fn test_location_new() {
        let loc = Location::new();
        assert_eq!(loc.as_str(), "");
    }

    #[test]
    fn test_location_join_property() {
        let loc = Location::new();
        let loc = loc.join("property");
        assert_eq!(loc.as_str(), "/property");
    }

    #[test]
    fn test_location_join_index() {
        let loc = Location::new();
        let loc = loc.join(0);
        assert_eq!(loc.as_str(), "/0");
    }

    #[test]
    fn test_location_join_multiple() {
        let loc = Location::new();
        let loc = loc.join("property").join(0);
        assert_eq!(loc.as_str(), "/property/0");
    }

    #[test]
    fn test_as_bytes() {
        let loc = Location::new().join("test");
        assert_eq!(loc.as_bytes(), b"/test");
    }

    #[test]
    fn test_display_trait() {
        let loc = Location::new().join("property");
        assert_eq!(format!("{loc}"), "/property");
    }

    #[test_case("tilde~character", "/tilde~0character"; "escapes tilde")]
    #[test_case("slash/character", "/slash~1character"; "escapes slash")]
    #[test_case("combo~and/slash", "/combo~0and~1slash"; "escapes tilde and slash combined")]
    #[test_case("multiple~/escapes~", "/multiple~0~1escapes~0"; "multiple escapes")]
    #[test_case("first/segment", "/first~1segment"; "escapes slash in nested segment")]
    fn test_location_escaping(segment: &str, expected: &str) {
        let loc = Location::new().join(segment);
        assert_eq!(loc.as_str(), expected);
    }

    #[test_case("/a/b/c", &[LocationSegment::from("a"), LocationSegment::from("b"), LocationSegment::from("c")]; "location with properties")]
    #[test_case("/1/2/3", &[LocationSegment::Index(1), LocationSegment::Index(2), LocationSegment::Index(3)]; "location with indices")]
    #[test_case("/a/1/b/2", &[
        LocationSegment::from("a"),
        LocationSegment::Index(1),
        LocationSegment::from("b"),
        LocationSegment::Index(2)
    ]; "mixed properties and indices")]
    fn test_into_iter(location: &str, expected_segments: &[LocationSegment]) {
        let loc = Location(Arc::from(location.to_string()));
        assert_eq!(loc.into_iter().collect::<Vec<_>>(), expected_segments);
    }

    #[test_case(vec![LocationSegment::from("a"), LocationSegment::from("b")], "/a/b"; "properties only")]
    #[test_case(vec![LocationSegment::Index(1), LocationSegment::Index(2)], "/1/2"; "indices only")]
    #[test_case(vec![LocationSegment::from("a"), LocationSegment::Index(1)], "/a/1"; "mixed segments")]
    fn test_from_iter(segments: Vec<LocationSegment>, expected: &str) {
        assert_eq!(Location::from_iter(segments).as_str(), expected);
    }

    #[test]
    fn test_roundtrip_join_iter_rebuild_equals() {
        let loc = Location::new().join("a/b").join(2).join("x~y");

        let segments: Vec<_> = loc.into_iter().collect();

        let rebuilt = segments
            .into_iter()
            .fold(Location::new(), |acc, seg| match seg {
                LocationSegment::Property(p) => acc.join(p),
                LocationSegment::Index(i) => acc.join(i),
            });

        assert_eq!(loc, rebuilt);
    }

    #[test]
    fn test_validate_error_instance_path_traverses_instance() {
        let schema = json!({
            "type": "object",
            "properties": {
                "table-node": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "~": { "type": "string", "minLength": 1 } },
                        "required": ["~"],
                    }
                }
            },
            "$schema": "https://json-schema.org/draft/2020-12/schema",
        });
        let instance = json!({
            "table-node": [
                { "~": "" },
                { "other-value": "" },
            ],
        });

        let error = crate::validate(&schema, &instance).expect_err("Should fail");

        // Traverse instance using the `instance_path`` segments
        let mut current = &instance;
        for segment in error.instance_path() {
            match segment {
                LocationSegment::Property(property) => {
                    current = &current[property.as_ref()];
                }
                LocationSegment::Index(idx) => {
                    current = &current[idx];
                }
            }
        }
        assert_eq!(
            current,
            instance
                .pointer("/table-node/0/~0")
                .expect("Pointer is valid")
        );
    }
}
