use crate::canonical::ir::{drop_subsumed, Bounds, Divisors, IntegerBounds, IntegerLeaf};

/// Integer leaves merged per divisor and free of subsumed intervals. Inserts are batched; the form
/// is restored before any read, so the order in which leaves arrive cannot change the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntegerLeaves {
    leaves: Vec<IntegerLeaf>,
    canonical: bool,
}

impl Default for IntegerLeaves {
    fn default() -> Self {
        Self {
            leaves: Vec::new(),
            canonical: true,
        }
    }
}

impl IntegerLeaves {
    pub(crate) fn insert(&mut self, leaf: IntegerLeaf) {
        self.leaves.push(leaf);
        self.canonical = false;
    }

    fn canonicalize(&mut self) {
        if self.canonical {
            return;
        }
        let was_empty = self.leaves.is_empty();
        self.leaves = merge(std::mem::take(&mut self.leaves));
        // A coarser progression over a wider interval admits every value of a finer one.
        // e.g.  anyOf [
        //         {"type": "integer", "multipleOf": 2},
        //         {"type": "integer", "multipleOf": 4}
        //       ]  =>  {"type": "integer", "multipleOf": 2}
        drop_subsumed(&mut self.leaves, |outer, inner| {
            outer.bounds.covers(&inner.bounds) && outer.multiple_of.divide_all(&inner.multiple_of)
        });
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
    pub(crate) fn retain(&mut self, keep: impl FnMut(&IntegerLeaf) -> bool) {
        self.canonicalize();
        self.leaves.retain(keep);
    }

    /// Merging never removes the last leaf, so this reads the batch without canonicalizing.
    pub(crate) fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub(crate) fn as_slice(&mut self) -> &[IntegerLeaf] {
        self.canonicalize();
        &self.leaves
    }
}

impl IntoIterator for IntegerLeaves {
    type Item = IntegerLeaf;
    type IntoIter = std::vec::IntoIter<IntegerLeaf>;

    fn into_iter(mut self) -> Self::IntoIter {
        self.canonicalize();
        self.leaves.into_iter()
    }
}

/// The divisor shared by a merge group; only the interval differs within one.
struct Group {
    divisor: Divisors,
}

/// Fold the intervals of leaves carrying the same divisor.
/// e.g.  anyOf [
///         {"type": "integer", "multipleOf": 2, "maximum": 10},
///         {"type": "integer", "multipleOf": 2, "minimum": 10}
///       ]  =>  {"type": "integer", "multipleOf": 2}
///
/// Different divisors admit different values, so those leaves stay apart.
/// e.g.  anyOf [
///         {"type": "integer", "multipleOf": 2},
///         {"type": "integer", "multipleOf": 3}
///       ]  =>  unchanged
fn merge(mut leaves: Vec<IntegerLeaf>) -> Vec<IntegerLeaf> {
    if leaves.len() < 2 {
        return leaves;
    }
    leaves.sort_by(|left, right| left.multiple_of.cmp(&right.multiple_of));
    let mut merged: Vec<IntegerLeaf> = Vec::with_capacity(leaves.len());
    let mut windows: Vec<IntegerBounds> = Vec::new();
    let mut group: Option<Group> = None;
    for leaf in leaves {
        if group
            .as_ref()
            .is_none_or(|open| open.divisor != leaf.multiple_of)
        {
            flush_group(&mut merged, group.take(), &mut windows);
            group = Some(Group {
                divisor: leaf.multiple_of,
            });
        }
        windows.push(leaf.bounds);
    }
    flush_group(&mut merged, group, &mut windows);
    merged
}

/// Emit one leaf per merged interval, all carrying the group's divisor. A gap between two intervals
/// keeps them as separate branches.
fn flush_group(
    merged: &mut Vec<IntegerLeaf>,
    group: Option<Group>,
    windows: &mut Vec<IntegerBounds>,
) {
    let Some(Group {
        divisor: multiple_of,
    }) = group
    else {
        return;
    };
    for bounds in Bounds::merge_all(std::mem::take(windows)) {
        merged.push(IntegerLeaf {
            bounds,
            multiple_of: multiple_of.clone(),
        });
    }
}
