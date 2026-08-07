use std::sync::Arc;

use crate::canonical::ir::{Bounds, LengthBounds, StringFormat, StringLeaf};

/// String leaves merged per pattern set and free of subsumed windows. Inserts are batched; the form
/// is restored before any read, so the order in which leaves arrive cannot change the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StringLeaves {
    leaves: Vec<StringLeaf>,
    canonical: bool,
}

impl Default for StringLeaves {
    fn default() -> Self {
        Self {
            leaves: Vec::new(),
            canonical: true,
        }
    }
}

impl StringLeaves {
    pub(crate) fn insert(&mut self, leaf: StringLeaf) {
        self.leaves.push(leaf);
        self.canonical = false;
    }

    fn canonicalize(&mut self) {
        if self.canonical {
            return;
        }
        let was_empty = self.leaves.is_empty();
        self.leaves = merge(std::mem::take(&mut self.leaves));
        drop_subsumed(&mut self.leaves);
        self.canonical = true;
        // `is_empty` reads the batch without canonicalizing, which relies on this.
        debug_assert_eq!(
            self.leaves.is_empty(),
            was_empty,
            "merging emptied the leaves"
        );
    }

    pub(crate) fn clear(&mut self) {
        self.leaves.clear();
        self.canonical = true;
    }

    /// Dropping leaves can neither make two of the rest mergeable nor subsume one by another.
    pub(crate) fn retain(&mut self, keep: impl FnMut(&StringLeaf) -> bool) {
        self.canonicalize();
        self.leaves.retain(keep);
    }

    /// Merging never removes the last leaf, so this reads the batch without canonicalizing.
    pub(crate) fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub(crate) fn as_slice(&mut self) -> &[StringLeaf] {
        self.canonicalize();
        &self.leaves
    }
}

impl IntoIterator for StringLeaves {
    type Item = StringLeaf;
    type IntoIter = std::vec::IntoIter<StringLeaf>;

    fn into_iter(mut self) -> Self::IntoIter {
        self.canonicalize();
        self.leaves.into_iter()
    }
}

/// The facets shared by a merge group; only the length window differs within one.
struct Facets {
    patterns: Vec<Arc<str>>,
    excluded_patterns: Vec<Arc<str>>,
    formats: Vec<StringFormat>,
    excluded_formats: Vec<StringFormat>,
    content_media_types: Vec<Arc<str>>,
    content_encodings: Vec<Arc<str>>,
    excluded: Vec<Arc<str>>,
}

/// Fold the length windows of leaves carrying the same patterns and formats.
/// e.g.  anyOf [
///         {"type": "string", "minLength": 3, "maxLength": 5},
///         {"type": "string", "minLength": 6, "maxLength": 9}
///       ]  =>  {"type": "string", "minLength": 3, "maxLength": 9}
///
/// Different patterns constrain different string sets, so those leaves stay apart.
/// e.g.  anyOf [
///         {"type": "string", "maxLength": 2, "pattern": "^a"},
///         {"type": "string", "minLength": 8}
///       ]  =>  unchanged
fn merge(mut leaves: Vec<StringLeaf>) -> Vec<StringLeaf> {
    if leaves.len() < 2 {
        return leaves;
    }
    leaves.sort_by(|left, right| {
        (
            &left.patterns,
            &left.excluded_patterns,
            &left.formats,
            &left.excluded_formats,
            &left.content_media_types,
            &left.content_encodings,
            &left.excluded,
        )
            .cmp(&(
                &right.patterns,
                &right.excluded_patterns,
                &right.formats,
                &right.excluded_formats,
                &right.content_media_types,
                &right.content_encodings,
                &right.excluded,
            ))
    });
    let mut merged: Vec<StringLeaf> = Vec::with_capacity(leaves.len());
    let mut windows: Vec<LengthBounds> = Vec::new();
    let mut facets: Option<Facets> = None;
    for leaf in leaves {
        if facets.as_ref().is_none_or(|group| {
            group.patterns != leaf.patterns
                || group.excluded_patterns != leaf.excluded_patterns
                || group.formats != leaf.formats
                || group.excluded_formats != leaf.excluded_formats
                || group.content_media_types != leaf.content_media_types
                || group.content_encodings != leaf.content_encodings
                || group.excluded != leaf.excluded
        }) {
            flush_group(&mut merged, facets.take(), &mut windows);
            facets = Some(Facets {
                patterns: leaf.patterns,
                excluded_patterns: leaf.excluded_patterns,
                formats: leaf.formats,
                excluded_formats: leaf.excluded_formats,
                content_media_types: leaf.content_media_types,
                content_encodings: leaf.content_encodings,
                excluded: leaf.excluded,
            });
        }
        windows.push(leaf.lengths);
    }
    flush_group(&mut merged, facets, &mut windows);
    merged
}

/// Emit one leaf per merged window, cloning the patterns onto each and moving them into the last.
/// A gap between two windows keeps them as separate branches, both still carrying the pattern.
/// e.g.  anyOf [
///         {"type": "string", "maxLength": 2, "pattern": "^a"},
///         {"type": "string", "minLength": 5, "pattern": "^a"}
///       ]  =>  unchanged
fn flush_group(
    merged: &mut Vec<StringLeaf>,
    facets: Option<Facets>,
    windows: &mut Vec<LengthBounds>,
) {
    let Some(Facets {
        patterns,
        excluded_patterns,
        formats,
        excluded_formats,
        content_media_types,
        content_encodings,
        excluded,
    }) = facets
    else {
        return;
    };
    let mut lengths = Bounds::merge_all(std::mem::take(windows));
    let last = lengths.pop().expect("a group holds at least one window");
    for window in lengths {
        merged.push(StringLeaf {
            lengths: window,
            patterns: patterns.clone(),
            excluded_patterns: excluded_patterns.clone(),
            formats: formats.clone(),
            excluded_formats: excluded_formats.clone(),
            content_media_types: content_media_types.clone(),
            content_encodings: content_encodings.clone(),
            excluded: excluded.clone(),
        });
    }
    merged.push(StringLeaf {
        lengths: last,
        patterns,
        excluded_patterns,
        formats,
        excluded_formats,
        content_media_types,
        content_encodings,
        excluded,
    });
}

/// Drop a leaf whose strings another leaf already admits: a window that contains it, under patterns
/// that constrain no more. Extra patterns only narrow, so the pattern-free leaf swallows the other.
/// e.g.  anyOf [
///         {"type": "string", "minLength": 2},
///         {"type": "string", "minLength": 2, "pattern": "^a"}
///       ]  =>  {"type": "string", "minLength": 2}
fn drop_subsumed(leaves: &mut Vec<StringLeaf>) {
    if leaves.len() < 2 {
        return;
    }
    let mut keep = vec![true; leaves.len()];
    for (index, leaf) in leaves.iter().enumerate() {
        for (other_index, other) in leaves.iter().enumerate() {
            if index == other_index || !keep[other_index] || !keep[index] {
                continue;
            }
            let wider = other.lengths.covers(&leaf.lengths)
                && other
                    .patterns
                    .iter()
                    .all(|pattern| leaf.patterns.contains(pattern))
                && other
                    .excluded_patterns
                    .iter()
                    .all(|pattern| leaf.excluded_patterns.contains(pattern))
                && other
                    .formats
                    .iter()
                    .all(|format| leaf.formats.contains(format))
                && other
                    .excluded_formats
                    .iter()
                    .all(|format| leaf.excluded_formats.contains(format))
                && other
                    .content_media_types
                    .iter()
                    .all(|media_type| leaf.content_media_types.contains(media_type))
                && other
                    .content_encodings
                    .iter()
                    .all(|encoding| leaf.content_encodings.contains(encoding))
                && other
                    .excluded
                    .iter()
                    .all(|value| leaf.excluded.contains(value));
            let facets = |leaf: &StringLeaf| {
                leaf.patterns.len()
                    + leaf.excluded_patterns.len()
                    + leaf.formats.len()
                    + leaf.excluded_formats.len()
                    + leaf.content_media_types.len()
                    + leaf.content_encodings.len()
                    + leaf.excluded.len()
            };
            if wider && (facets(other) < facets(leaf) || index > other_index) {
                keep[index] = false;
            }
        }
    }
    let mut index = 0;
    leaves.retain(|_| {
        let keeps = keep[index];
        index += 1;
        keeps
    });
}
