use crate::canonical::ir::{NumberLeaf, Side};

/// Number intervals kept sorted and pairwise unmergeable. One interval containing another also
/// overlaps it, so folding leaves nothing for a separate subsumption pass to drop. Inserts are batched; the form is restored
/// before any read, so the order in which intervals arrive cannot change the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NumberLeaves {
    leaves: Vec<NumberLeaf>,
    canonical: bool,
}

impl Default for NumberLeaves {
    fn default() -> Self {
        Self {
            leaves: Vec::new(),
            canonical: true,
        }
    }
}

impl NumberLeaves {
    pub(crate) fn insert(&mut self, leaf: NumberLeaf) {
        self.leaves.push(leaf);
        self.canonical = false;
    }

    fn canonicalize(&mut self) {
        if self.canonical {
            return;
        }
        let was_empty = self.leaves.is_empty();
        self.leaves = merge(std::mem::take(&mut self.leaves));
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

    /// Dropping intervals can neither reorder the rest nor make two of them mergeable.
    pub(crate) fn retain(&mut self, keep: impl FnMut(&NumberLeaf) -> bool) {
        self.canonicalize();
        self.leaves.retain(keep);
    }

    /// Merging never removes the last interval, so this reads the batch without canonicalizing.
    pub(crate) fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub(crate) fn as_slice(&mut self) -> &[NumberLeaf] {
        self.canonicalize();
        &self.leaves
    }
}

impl IntoIterator for NumberLeaves {
    type Item = NumberLeaf;
    type IntoIter = std::vec::IntoIter<NumberLeaf>;

    fn into_iter(mut self) -> Self::IntoIter {
        self.canonicalize();
        self.leaves.into_iter()
    }
}

/// Fold intervals that overlap or meet on an admitted point.
/// e.g.  anyOf [
///         {"type": "number", "minimum": 0, "maximum": 2},
///         {"type": "number", "minimum": 2, "maximum": 4}
///       ]  =>  {"type": "number", "minimum": 0, "maximum": 4}
///
/// Two intervals meeting on a point neither admits leave a hole, so they stay apart.
/// e.g.  anyOf [
///         {"type": "number", "maximum": 2, "exclusiveMaximum": 2},
///         {"type": "number", "exclusiveMinimum": 2}
///       ]  =>  unchanged
fn merge(mut leaves: Vec<NumberLeaf>) -> Vec<NumberLeaf> {
    if leaves.len() < 2 {
        return leaves;
    }
    // Sort by where each interval starts: an absent minimum is unbounded below, and on a shared
    // limit the inclusive end starts earlier than the excluded one.
    leaves.sort_by(|left, right| match (&left.minimum, &right.minimum) {
        (Some(left), Some(right)) if left.to_number() == right.to_number() => {
            right.is_inclusive().cmp(&left.is_inclusive())
        }
        (left, right) => left.cmp(right),
    });
    let mut merged: Vec<NumberLeaf> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        match merged.last_mut() {
            Some(last) if reaches(last, &leaf) => *last = hull(std::mem::take(last), leaf),
            _ => merged.push(leaf),
        }
    }
    merged
}

/// Whether the two leave no real value between them. `next` starts no lower than `last`.
fn reaches(last: &NumberLeaf, next: &NumberLeaf) -> bool {
    let (Some(end), Some(start)) = (&last.maximum, &next.minimum) else {
        return true;
    };
    if end.to_number() == start.to_number() {
        // They meet on one point, which closes the gap only if either side admits it.
        return end.is_inclusive() || start.is_inclusive();
    }
    end.admits(&start.to_number(), Side::Upper)
}

/// The narrowest interval holding both. An absent bound is unbounded, so it swallows the present one.
fn hull(last: NumberLeaf, next: NumberLeaf) -> NumberLeaf {
    NumberLeaf {
        minimum: match (last.minimum, next.minimum) {
            (Some(left), Some(right)) => Some(if left.is_tighter_than(&right, Side::Lower) {
                right
            } else {
                left
            }),
            _ => None,
        },
        maximum: match (last.maximum, next.maximum) {
            (Some(left), Some(right)) => Some(if left.is_tighter_than(&right, Side::Upper) {
                right
            } else {
                left
            }),
            _ => None,
        },
    }
}
