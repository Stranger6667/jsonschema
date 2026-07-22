use crate::canonical::ir::{drop_subsumed, BoundNumber, NumberLeaf, Side};

/// Number leaves kept sorted, pairwise unmergeable, and free of leaves another already admits.
/// Inserts are batched; the form is restored before any read, so the order in which leaves arrive
/// cannot change the result.
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
        // A coarser progression over a wider interval admits every value of a finer one.
        // e.g.  anyOf [
        //         {"type": "number", "multipleOf": 0.5},
        //         {"type": "number", "multipleOf": 1.5}
        //       ]  =>  {"type": "number", "multipleOf": 0.5}
        drop_subsumed(&mut self.leaves, |outer, inner| {
            covers(outer, inner) && outer.multiple_of.divide_all(&inner.multiple_of)
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
///
/// Different divisors admit different values within one interval, so those leaves stay apart too.
/// e.g.  anyOf [
///         {"type": "number", "multipleOf": 0.5},
///         {"type": "number", "multipleOf": 0.75}
///       ]  =>  unchanged
fn merge(mut leaves: Vec<NumberLeaf>) -> Vec<NumberLeaf> {
    if leaves.len() < 2 {
        return leaves;
    }
    // Sort by divisor first so leaves sharing one are adjacent, then by where each interval starts:
    // an absent minimum is unbounded below, and on a shared limit the inclusive end starts earlier
    // than the excluded one.
    leaves.sort_by(|left, right| {
        left.multiple_of.cmp(&right.multiple_of).then_with(|| {
            match (&left.minimum, &right.minimum) {
                (Some(left), Some(right)) if left.to_number() == right.to_number() => {
                    right.is_inclusive().cmp(&left.is_inclusive())
                }
                (left, right) => left.cmp(right),
            }
        })
    });
    let mut merged: Vec<NumberLeaf> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        match merged.last_mut() {
            Some(last) if last.multiple_of == leaf.multiple_of && reaches(last, &leaf) => {
                *last = hull(std::mem::take(last), leaf);
            }
            _ => merged.push(leaf),
        }
    }
    merged
}

/// Whether `outer` admits every real value `inner` does, divisors aside.
fn covers(outer: &NumberLeaf, inner: &NumberLeaf) -> bool {
    // Equal ends are tighter than each other, so only a strictly tighter outer end fails to cover.
    let wider = |outer: &BoundNumber, inner: &BoundNumber, side| {
        !outer.is_tighter_than(inner, side) || inner.is_tighter_than(outer, side)
    };
    let minimum = match (&outer.minimum, &inner.minimum) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(outer), Some(inner)) => wider(outer, inner, Side::Lower),
    };
    let maximum = match (&outer.maximum, &inner.maximum) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(outer), Some(inner)) => wider(outer, inner, Side::Upper),
    };
    minimum && maximum
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
    // Sorting put the wider minimum first, which is what lets `last` keep it unexamined.
    debug_assert!(
        !(last.minimum.is_some() && next.minimum.is_none()),
        "an unbounded minimum sorted after a bounded one"
    );
    // Equal bounds are tighter than each other, so only a strictly tighter one is out of order.
    debug_assert!(
        !matches!((&last.minimum, &next.minimum), (Some(left), Some(right))
            if left.is_tighter_than(right, Side::Lower)
                && !right.is_tighter_than(left, Side::Lower)),
        "the tighter minimum sorted first"
    );
    // `merge` folds only leaves carrying the same divisor, so `last` speaks for both.
    debug_assert_eq!(
        last.multiple_of, next.multiple_of,
        "folding intervals under different divisors"
    );
    NumberLeaf {
        multiple_of: last.multiple_of,
        minimum: last.minimum,
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
