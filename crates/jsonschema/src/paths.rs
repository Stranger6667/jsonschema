//! Facilities for working with paths within schemas or validated instances.
use std::{
    borrow::Cow,
    fmt,
    sync::{Arc, OnceLock},
};

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

/// Cached empty location - very common for root-level errors.
static EMPTY_LOCATION: OnceLock<Location> = OnceLock::new();

/// Cached single-index paths for indices 0-15.
/// Cloning these is just an atomic increment (Arc clone).
static CACHED_INDEX_PATHS: OnceLock<[Location; 16]> = OnceLock::new();

#[inline]
fn get_cached_index_paths() -> &'static [Location; 16] {
    CACHED_INDEX_PATHS.get_or_init(|| {
        std::array::from_fn(|i| {
            let mut itoa_buf = itoa::Buffer::new();
            let s = itoa_buf.format(i);
            let mut buffer = String::with_capacity(1 + s.len());
            buffer.push('/');
            buffer.push_str(s);
            Location(Arc::from(buffer))
        })
    })
}

impl<'a> From<&'a LazyLocation<'_, '_>> for Location {
    fn from(value: &'a LazyLocation<'_, '_>) -> Self {
        const STACK_CAPACITY: usize = 16;

        // Fast path: empty location
        if value.parent.is_none() {
            return Location::new();
        }

        // Fast path: single index segment (very common for array validation)
        // Use cached locations for indices 0-15 to avoid allocation
        if let Some(parent) = value.parent {
            if parent.parent.is_none() {
                if let LocationSegment::Index(idx) = &value.segment {
                    if *idx < 16 {
                        return get_cached_index_paths()[*idx].clone();
                    }
                    // Single index > 15: compute directly
                    let mut buf = itoa::Buffer::new();
                    return Location(Arc::from(format!("/{}", buf.format(*idx))));
                }
            }
        }

        // General path: multi-segment

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

/// Tracks `$ref` traversals during validation for evaluation path computation.
///
/// This is a stack-allocated linked list that gets pushed when crossing `$ref` boundaries.
/// Each entry stores the **suffix** of the `$ref` location (path relative to its resource base),
/// which is precomputed at compile time.
///
/// Use `Option<&RefTracker>` throughout the validation code:
/// - `None` means no `$ref` has been traversed yet
/// - `Some(&tracker)` means at least one `$ref` has been crossed
///
/// # Example: Single $ref
///
/// ```text
/// Schema:
/// {
///   "properties": {
///     "user": { "$ref": "#/$defs/Person" }
///   },
///   "$defs": {
///     "Person": { "type": "object" }
///   }
/// }
///
/// Instance: { "user": "not-an-object" }
///
/// At the "type" validator:
///   tracker.prefix() = /properties/user/$ref
///   validator.suffix = /type
///   tracker  = /properties/user/$ref/type
/// ```
///
/// # Example: Nested $refs
///
/// ```text
/// Schema:
/// {
///   "properties": {
///     "order": { "$ref": "#/$defs/Order" }
///   },
///   "$defs": {
///     "Order": {
///       "properties": {
///         "item": { "$ref": "#/$defs/Item" }
///       }
///     },
///     "Item": { "type": "string" }
///   }
/// }
///
/// Instance: { "order": { "item": 123 } }
///
/// At the "type" validator:
///   tracker has two entries:
///     1. suffix = /properties/order/$ref (from root resource)
///     2. suffix = /properties/item/$ref  (from $defs/Order resource)
///   tracker.prefix() = /properties/order/$ref/properties/item/$ref
///   validator.suffix = /type
///   tracker  = /properties/order/$ref/properties/item/$ref/type
/// ```
#[derive(Debug)]
pub(crate) struct RefTracker<'a> {
    /// Path of the `$ref` keyword relative to its resource base.
    /// E.g., `/properties/user/$ref` (not the full canonical path).
    suffix: &'a Location,
    /// The resource base of the `$ref` target.
    /// Used to compute validator suffixes at runtime.
    /// E.g., `/$defs/Person` when `$ref` points to `#/$defs/Person`.
    target_base: &'a Location,
    /// Parent tracker for nested `$ref`s. `None` for the first `$ref` in the chain.
    parent: Option<&'a RefTracker<'a>>,
    /// Cached joined prefix (computed once on first access).
    cached_prefix: std::sync::OnceLock<Location>,
}

impl<'a> RefTracker<'a> {
    /// Create a new tracker for a `$ref` traversal.
    ///
    /// # Arguments
    /// - `suffix`: The `$ref` keyword's path relative to its resource base
    ///   (precomputed at compile time via `ctx.suffix()`)
    /// - `target_base`: The resource base of the `$ref` target
    ///   (e.g., `/$defs/Person` when `$ref` points to `#/$defs/Person`)
    /// - `parent`: The parent tracker, or `None` if this is the first `$ref`
    #[inline]
    #[must_use]
    pub(crate) fn new(
        suffix: &'a Location,
        target_base: &'a Location,
        parent: Option<&'a RefTracker<'a>>,
    ) -> Self {
        RefTracker {
            suffix,
            target_base,
            parent,
            cached_prefix: std::sync::OnceLock::new(),
        }
    }

    /// Get the joined prefix of all `$ref` suffixes.
    ///
    /// Computed once on first access, then cached.
    #[inline]
    pub(crate) fn prefix(&self) -> &Location {
        self.cached_prefix.get_or_init(|| self.compute_prefix())
    }

    /// Compute the suffix for a canonical location by stripping the `target_base`.
    ///
    /// E.g., for location `/$defs/Person/type` with `target_base` `/$defs/Person`,
    /// returns `/type`.
    #[inline]
    pub(crate) fn compute_suffix(&self, location: &Location) -> Location {
        let suffix = location
            .as_str()
            .strip_prefix(self.target_base.as_str())
            .unwrap_or(location.as_str());
        Location::from_escaped(suffix)
    }

    /// Compute the evaluation path for a validator with the given canonical location.
    ///
    /// This is: `prefix() + (location - target_base)`
    #[inline]
    pub(crate) fn evaluation_path(&self, location: &Location) -> Location {
        let suffix = location
            .as_str()
            .strip_prefix(self.target_base.as_str())
            .unwrap_or(location.as_str());
        self.prefix().join_raw_suffix(suffix)
    }

    fn compute_prefix(&self) -> Location {
        match self.parent {
            None => self.suffix.clone(),
            Some(parent) => parent.prefix().join_raw_suffix(self.suffix.as_str()),
        }
    }

    /// Capture evaluation path state at error creation time.
    ///
    /// Returns a lazy evaluation path that defers computation until needed.
    #[inline]
    pub(crate) fn capture(&self, location: &Location) -> LazyEvaluationPath {
        LazyEvaluationPath::Deferred {
            prefix: self.prefix().clone(),
            suffix: self.compute_suffix(location),
            cached: OnceLock::new(),
        }
    }
}

/// Compute the evaluation path, using the tracker if present.
#[inline]
pub(crate) fn evaluation_path(tracker: Option<&RefTracker<'_>>, location: &Location) -> Location {
    match tracker {
        None => location.clone(),
        Some(t) => t.evaluation_path(location),
    }
}

/// Capture evaluation path state at error creation time.
#[inline]
pub(crate) fn capture_evaluation_path(
    tracker: Option<&RefTracker<'_>>,
    location: &Location,
) -> LazyEvaluationPath {
    match tracker {
        None => LazyEvaluationPath::SameAsSchemaPath,
        Some(t) => t.capture(location),
    }
}

/// Lazily-computed evaluation path stored in validation errors.
///
/// # Why lazy?
///
/// Validators like `anyOf` collect errors from all branches but discard them
/// if any branch succeeds. Eagerly computing evaluation paths for discarded
/// errors wastes CPU cycles. We defer the string join until the error is
/// actually displayed.
///
/// # Example 1: No $ref traversal
///
/// ```text
/// Schema: { "properties": { "name": { "type": "string" } } }
/// Instance: { "name": 123 }
///
/// schema_path = tracker = /properties/name/type
///
/// We store: SameAsSchemaPath (zero extra allocation)
/// ```
///
/// # Example 2: Single $ref
///
/// ```text
/// Schema:
/// {
///   "properties": {
///     "user": { "$ref": "#/$defs/Person" }
///   },
///   "$defs": {
///     "Person": { "type": "object" }
///   }
/// }
/// Instance: { "user": "not-an-object" }
///
/// schema_path     = /$defs/Person/type  (canonical location, no $ref)
/// tracker = /properties/user/$ref/type (actual traversal path)
///
/// We store:
///   - prefix: /properties/user/$ref  (from RefTracker)
///   - suffix: /type                  (precomputed at compile time)
///
/// On access: tracker = prefix + suffix
/// ```
///
/// # Example 3: Nested $refs
///
/// ```text
/// Schema:
/// {
///   "properties": {
///     "order": { "$ref": "#/$defs/Order" }
///   },
///   "$defs": {
///     "Order": {
///       "properties": {
///         "item": { "$ref": "#/$defs/Item" }
///       }
///     },
///     "Item": {
///       "properties": {
///         "price": { "type": "number" }
///       }
///     }
///   }
/// }
/// Instance: { "order": { "item": { "price": "free" } } }
///
/// schema_path     = /$defs/Item/properties/price/type
/// tracker = /properties/order/$ref/properties/item/$ref/properties/price/type
///
/// We store:
///   - prefix: /properties/order/$ref/properties/item/$ref  (from RefTracker chain)
///   - suffix: /properties/price/type                       (computed at error creation)
/// ```
#[derive(Debug)]
pub(crate) enum LazyEvaluationPath {
    /// No `$ref` traversal - `tracker` equals `schema_path`.
    /// Zero extra storage; we reference `ValidationError::schema_path` directly.
    SameAsSchemaPath,

    /// Already computed evaluation path (e.g., from `into_owned`).
    Computed(Location),

    /// `$ref` was traversed - lazily compute `prefix + suffix`.
    Deferred {
        /// Chain of `$ref` locations (e.g., `/properties/user/$ref`)
        prefix: Location,
        /// Path relative to innermost `$ref` target (e.g., `/type`)
        suffix: Location,
        /// Cached result of `prefix.join_raw_suffix(&suffix)`
        cached: OnceLock<Location>,
    },
}

impl Clone for LazyEvaluationPath {
    fn clone(&self) -> Self {
        match self {
            LazyEvaluationPath::SameAsSchemaPath => LazyEvaluationPath::SameAsSchemaPath,
            LazyEvaluationPath::Computed(loc) => LazyEvaluationPath::Computed(loc.clone()),
            LazyEvaluationPath::Deferred {
                prefix,
                suffix,
                cached,
            } => {
                let new_cached = OnceLock::new();
                if let Some(val) = cached.get() {
                    let _ = new_cached.set(val.clone());
                }
                LazyEvaluationPath::Deferred {
                    prefix: prefix.clone(),
                    suffix: suffix.clone(),
                    cached: new_cached,
                }
            }
        }
    }
}

impl From<Location> for LazyEvaluationPath {
    /// Convert a computed Location to `LazyEvaluationPath`.
    ///
    /// Used when an evaluation path has been computed (e.g., via `into_owned`) and needs
    /// to be preserved when creating a new error.
    #[inline]
    fn from(location: Location) -> Self {
        LazyEvaluationPath::Computed(location)
    }
}

impl LazyEvaluationPath {
    /// Resolve the evaluation path, computing it if necessary.
    ///
    /// For `SameAsSchemaPath`, caller must pass the `schema_path` reference.
    #[inline]
    #[must_use]
    pub(crate) fn resolve<'a>(&'a self, schema_path: &'a Location) -> &'a Location {
        match self {
            LazyEvaluationPath::SameAsSchemaPath => schema_path,
            LazyEvaluationPath::Computed(loc) => loc,
            LazyEvaluationPath::Deferred {
                prefix,
                suffix,
                cached,
            } => cached.get_or_init(|| prefix.join_raw_suffix(suffix.as_str())),
        }
    }

    /// Consume self and return the owned evaluation path.
    #[inline]
    #[must_use]
    pub(crate) fn into_owned(self, schema_path: Location) -> Location {
        match self {
            LazyEvaluationPath::SameAsSchemaPath => schema_path,
            LazyEvaluationPath::Computed(loc) => loc,
            LazyEvaluationPath::Deferred {
                prefix,
                suffix,
                cached,
            } => cached
                .into_inner()
                .unwrap_or_else(|| prefix.join_raw_suffix(suffix.as_str())),
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
    ///
    /// Returns a cached instance to avoid allocation.
    #[must_use]
    pub fn new() -> Self {
        EMPTY_LOCATION.get_or_init(|| Self(Arc::from(""))).clone()
    }

    pub(crate) fn from_escaped(escaped: &str) -> Self {
        Self(Arc::from(escaped))
    }

    /// Append a raw JSON pointer suffix (already escaped).
    /// This is more efficient than multiple `join` calls when the suffix
    /// is already a valid JSON pointer path.
    #[must_use]
    pub(crate) fn join_raw_suffix(&self, suffix: &str) -> Self {
        debug_assert!(!suffix.is_empty(), "suffix should never be empty");
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
                Self(format!("{parent}/{segment}").into())
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

    #[test_case(0, "/0"; "cached index 0")]
    #[test_case(15, "/15"; "cached index 15")]
    #[test_case(16, "/16"; "uncached index 16")]
    #[test_case(100, "/100"; "uncached index 100")]
    fn test_lazy_location_single_index(idx: usize, expected: &str) {
        let root = LazyLocation::new();
        let loc = root.push(idx);
        let location: Location = (&loc).into();
        assert_eq!(location.as_str(), expected);
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

    #[test]
    fn test_deep_path_heap_allocation() {
        // Create a schema with >16 levels of nesting to exercise heap allocation path
        const DEPTH: usize = 20;

        let mut schema = json!({"type": "integer"});
        for i in (0..DEPTH).rev() {
            schema = json!({
                "type": "object",
                "properties": {
                    format!("level{i}"): schema
                }
            });
        }

        let mut instance = json!("not an integer");
        for i in (0..DEPTH).rev() {
            instance = json!({
                format!("level{i}"): instance
            });
        }

        let error = crate::validate(&schema, &instance).expect_err("Should fail");

        // Verify instance_path has correct depth (>16 exercises heap allocation)
        let instance_path = error.instance_path();
        assert_eq!(instance_path.into_iter().count(), DEPTH);
        let expected_instance_path = (0..DEPTH).fold(String::new(), |mut acc, i| {
            use std::fmt::Write;
            write!(acc, "/level{i}").unwrap();
            acc
        });
        assert_eq!(instance_path.as_str(), expected_instance_path);

        // Verify schema_path also has correct depth
        let schema_path = error.schema_path();
        let expected_schema_path = (0..DEPTH).fold(String::new(), |mut acc, i| {
            use std::fmt::Write;
            write!(acc, "/properties/level{i}").unwrap();
            acc
        }) + "/type";
        assert_eq!(schema_path.as_str(), expected_schema_path);
    }

    #[test]
    fn test_dynamic_ref_evaluation_path() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$dynamicAnchor": "node",
            "type": "object",
            "properties": {
                "data": {"type": "string"},
                "child": {"$dynamicRef": "#node"}
            }
        });
        let instance = json!({
            "data": "parent",
            "child": {
                "data": 123
            }
        });

        let error = crate::validate(&schema, &instance).expect_err("Should fail");

        // schema_path is canonical (anchor resolves to root)
        assert_eq!(error.schema_path().as_str(), "/properties/data/type");
        // evaluation_path includes $dynamicRef traversal
        assert_eq!(
            error.evaluation_path().as_str(),
            "/properties/child/$dynamicRef/properties/data/type"
        );
        assert_eq!(error.instance_path().as_str(), "/child/data");
    }

    #[test]
    fn test_nested_ref_evaluation_path() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "properties": {
                "order": { "$ref": "#/$defs/Order" }
            },
            "$defs": {
                "Order": {
                    "properties": {
                        "item": { "$ref": "#/$defs/Item" }
                    }
                },
                "Item": { "type": "string" }
            }
        });
        let instance = json!({ "order": { "item": 123 } });

        let error = crate::validate(&schema, &instance).expect_err("Should fail");

        assert_eq!(error.schema_path().as_str(), "/$defs/Item/type");
        assert_eq!(
            error.evaluation_path().as_str(),
            "/properties/order/$ref/properties/item/$ref/type"
        );
        assert_eq!(error.instance_path().as_str(), "/order/item");
    }

    #[test]
    fn test_no_ref_evaluation_path_equals_schema_path() {
        let schema = json!({
            "properties": {
                "name": { "type": "string" }
            }
        });
        let instance = json!({ "name": 123 });

        let error = crate::validate(&schema, &instance).expect_err("Should fail");

        assert_eq!(error.schema_path().as_str(), "/properties/name/type");
        assert_eq!(
            error.evaluation_path().as_str(),
            error.schema_path().as_str()
        );
    }
}
