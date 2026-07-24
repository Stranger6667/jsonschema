use crate::canonical::ir::{ArrayLeaf, BoundCardinality, Bounds, LengthBounds, Schema};

/// Array leaves merged per uniqueness flag and item schema, free of subsumed windows. Inserts are
/// batched; the form is restored before any read, so the order in which leaves arrive cannot
/// change the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArrayLeaves {
    leaves: Vec<ArrayLeaf>,
    canonical: bool,
}

impl Default for ArrayLeaves {
    fn default() -> Self {
        Self {
            leaves: Vec::new(),
            canonical: true,
        }
    }
}

impl ArrayLeaves {
    pub(crate) fn insert(&mut self, leaf: ArrayLeaf) {
        self.leaves.push(leaf);
        self.canonical = false;
    }

    fn canonicalize(&mut self) {
        if self.canonical {
            return;
        }
        let was_empty = self.leaves.is_empty();
        self.leaves = merge(std::mem::take(&mut self.leaves));
        extend_over_bare_windows(&mut self.leaves);
        hand_off_empty(&mut self.leaves);
        // Extending can overlap the windows of leaves sharing their facets; fold those again.
        self.leaves = merge(std::mem::take(&mut self.leaves));
        absorb_trivially_distinct(&mut self.leaves);
        absorb_trivially_conforming(&mut self.leaves);
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
    pub(crate) fn retain(&mut self, keep: impl FnMut(&ArrayLeaf) -> bool) {
        self.canonicalize();
        self.leaves.retain(keep);
    }

    /// Merging never removes the last leaf, so this reads the batch without canonicalizing.
    pub(crate) fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub(crate) fn as_slice(&mut self) -> &[ArrayLeaf] {
        self.canonicalize();
        &self.leaves
    }
}

impl IntoIterator for ArrayLeaves {
    type Item = ArrayLeaf;
    type IntoIter = std::vec::IntoIter<ArrayLeaf>;

    fn into_iter(mut self) -> Self::IntoIter {
        self.canonicalize();
        self.leaves.into_iter()
    }
}

/// Fold the length windows of leaves that agree on uniqueness and the item schema.
/// e.g.  anyOf [
///         {"type": "array", "uniqueItems": true, "maxItems": 2},
///         {"type": "array", "uniqueItems": true, "minItems": 3}
///       ]  =>  {"type": "array", "uniqueItems": true}
///
/// One demanding distinct items admits fewer arrays, so those leaves stay apart.
/// e.g.  anyOf [
///         {"type": "array", "uniqueItems": true, "minItems": 5},
///         {"type": "array", "maxItems": 2}
///       ]  =>  unchanged
fn merge(mut leaves: Vec<ArrayLeaf>) -> Vec<ArrayLeaf> {
    if leaves.len() < 2 {
        return leaves;
    }
    leaves.sort_by(|left, right| {
        (left.unique, &left.prefix, &left.items).cmp(&(right.unique, &right.prefix, &right.items))
    });
    let mut merged: Vec<ArrayLeaf> = Vec::with_capacity(leaves.len());
    let mut windows: Vec<LengthBounds> = Vec::new();
    let mut facets: Option<Facets> = None;
    for leaf in leaves {
        if facets.as_ref().is_none_or(|group| {
            group.unique != leaf.unique || group.prefix != leaf.prefix || group.items != leaf.items
        }) {
            flush_group(&mut merged, facets.take(), &mut windows);
            facets = Some(Facets {
                unique: leaf.unique,
                prefix: leaf.prefix,
                items: leaf.items,
            });
        }
        windows.push(leaf.lengths);
    }
    flush_group(&mut merged, facets, &mut windows);
    merged
}

/// The facets shared by a merge group; only the length window differs within one.
struct Facets {
    unique: bool,
    prefix: Vec<Schema>,
    items: Option<Schema>,
}

/// Emit one leaf per merged window, all carrying the group's facets.
fn flush_group(
    merged: &mut Vec<ArrayLeaf>,
    facets: Option<Facets>,
    windows: &mut Vec<LengthBounds>,
) {
    let Some(Facets {
        unique,
        prefix,
        items,
    }) = facets
    else {
        return;
    };
    let mut lengths = Bounds::merge_all(std::mem::take(windows));
    let last = lengths.pop().expect("a group holds at least one window");
    for window in lengths {
        merged.push(ArrayLeaf {
            lengths: window,
            unique,
            prefix: prefix.clone(),
            items: items.clone(),
        });
    }
    merged.push(ArrayLeaf {
        lengths: last,
        unique,
        prefix,
        items,
    });
}

/// Widen a facet-carrying window over a bare sibling window it touches: the lengths gained lie
/// inside the bare window, which admits those arrays with any content, so the union is unchanged.
/// The boundary between the two then has one spelling, whatever the facet leaf's window said.
/// ```text
/// e.g.  anyOf [
///         {"type": "array", "maxItems": 1},
///         {"type": "array", "items": {"type": "integer"}, "minItems": 2}
///       ]  =>  anyOf [
///         {"type": "array", "maxItems": 1},
///         {"type": "array", "items": {"type": "integer"}}
///       ]
/// ```
fn extend_over_bare_windows(leaves: &mut [ArrayLeaf]) {
    let bare: Vec<LengthBounds> = leaves
        .iter()
        .filter(|leaf| !leaf.unique && leaf.prefix.is_empty() && leaf.items.is_none())
        .map(|leaf| leaf.lengths.clone())
        .collect();
    if bare.is_empty() {
        return;
    }
    for leaf in leaves.iter_mut() {
        if !leaf.unique && leaf.prefix.is_empty() && leaf.items.is_none() {
            continue;
        }
        // A grown window can reach the next bare window, so retry until none applies.
        loop {
            let mut grown = false;
            for window in &bare {
                let merged = Bounds::merge_all(vec![leaf.lengths.clone(), window.clone()]);
                if let Ok([merged]) = <[_; 1]>::try_from(merged) {
                    if merged != leaf.lengths {
                        leaf.lengths = merged;
                        grown = true;
                    }
                }
            }
            if !grown {
                break;
            }
        }
    }
}

/// Drop a `minItems: 1` when another branch admits the empty array: the drop adds only `[]`, which
/// that branch accepts and which satisfies any item schema and distinctness vacuously.
/// ```text
/// e.g.  anyOf [
///         {"type": "array", "uniqueItems": true, "minItems": 1},
///         {"type": "array", "items": {"type": "integer"}}
///       ]  =>  anyOf [
///         {"type": "array", "uniqueItems": true},
///         {"type": "array", "items": {"type": "integer"}}
///       ]
/// ```
fn hand_off_empty(leaves: &mut [ArrayLeaf]) {
    if !leaves.iter().any(|leaf| {
        leaf.lengths
            .minimum
            .as_ref()
            .is_none_or(BoundCardinality::is_zero)
    }) {
        return;
    }
    let one = BoundCardinality::from(1);
    for leaf in leaves.iter_mut() {
        if leaf.lengths.minimum.as_ref() == Some(&one) {
            leaf.lengths.minimum = None;
        }
    }
}

/// A window of at most one item holds nothing that can repeat, so its arrays are distinct already:
/// widen a neighbouring leaf that demands distinctness over it.
/// e.g.  anyOf [
///         {"type": "array", "maxItems": 1},
///         {"type": "array", "uniqueItems": true, "minItems": 2}
///       ]  =>  {"type": "array", "uniqueItems": true}
///
/// A gap between the two leaves keeps them apart, since the lengths between them admit repeats.
/// e.g.  anyOf [
///         {"type": "array", "maxItems": 1},
///         {"type": "array", "uniqueItems": true, "minItems": 4}
///       ]  =>  unchanged
fn absorb_trivially_distinct(leaves: &mut Vec<ArrayLeaf>) {
    let Some(trivial) = leaves.iter().position(|leaf| {
        !leaf.unique
            && leaf.prefix.is_empty()
            && leaf.items.is_none()
            && leaf
                .lengths
                .maximum
                .as_ref()
                .is_some_and(|max| *max <= BoundCardinality::from(1))
    }) else {
        return;
    };
    let window = leaves[trivial].lengths.clone();
    // Merging the pair yields one window exactly when they overlap or touch. An element-constrained
    // leaf cannot widen over arrays of one item, whose element it never checked.
    let Some((target, widened)) = leaves.iter().enumerate().find_map(|(index, leaf)| {
        if !leaf.unique || leaf.items.is_some() || !leaf.prefix.is_empty() {
            return None;
        }
        let mut merged = Bounds::merge_all(vec![leaf.lengths.clone(), window.clone()]);
        (merged.len() == 1).then(|| (index, merged.pop().expect("a merged window")))
    }) else {
        return;
    };
    leaves[target].lengths = widened;
    leaves.remove(trivial);
}

/// A window of no items holds only the empty array, whose elements satisfy any item schema
/// vacuously: widen a neighbouring item-constrained leaf over it.
/// ```text
/// e.g.  anyOf [
///         {"const": []},
///         {"type": "array", "items": {"type": "integer"}, "minItems": 1}
///       ]  =>  {"type": "array", "items": {"type": "integer"}}
/// ```
///
/// A gap between the two windows keeps them apart: widening would admit the lengths between them.
/// ```text
/// e.g.  anyOf [
///         {"const": []},
///         {"type": "array", "items": {"type": "integer"}, "minItems": 2}
///       ]  =>  unchanged
/// ```
fn absorb_trivially_conforming(leaves: &mut Vec<ArrayLeaf>) {
    let Some(trivial) = leaves.iter().position(|leaf| {
        !leaf.unique
            && leaf.prefix.is_empty()
            && leaf.items.is_none()
            && leaf
                .lengths
                .maximum
                .as_ref()
                .is_some_and(BoundCardinality::is_zero)
    }) else {
        return;
    };
    let window = leaves[trivial].lengths.clone();
    // Merging the pair yields one window exactly when they overlap or touch.
    let Some((target, widened)) = leaves
        .iter()
        .enumerate()
        .filter(|(_, leaf)| leaf.items.is_some() || !leaf.prefix.is_empty())
        .find_map(|(index, leaf)| {
            let merged = Bounds::merge_all(vec![leaf.lengths.clone(), window.clone()]);
            match <[_; 1]>::try_from(merged) {
                Ok([widened]) => Some((index, widened)),
                Err(_) => None,
            }
        })
    else {
        return;
    };
    leaves[target].lengths = widened;
    leaves.remove(trivial);
}

/// Drop a leaf whose arrays another leaf already admits: a window that contains it, not demanding
/// distinctness unless it does too.
/// e.g.  anyOf [
///         {"type": "array"},
///         {"type": "array", "uniqueItems": true}
///       ]  =>  {"type": "array"}
fn drop_subsumed(leaves: &mut Vec<ArrayLeaf>) {
    if leaves.len() < 2 {
        return;
    }
    let mut keep = vec![true; leaves.len()];
    for (index, leaf) in leaves.iter().enumerate() {
        for (other_index, other) in leaves.iter().enumerate() {
            if index == other_index || !keep[other_index] || !keep[index] {
                continue;
            }
            // Element constraints are compared only for equality: deciding that one schema admits
            // every element another does needs a subsumption test the algebra does not have. So
            // `other` is looser exactly when its per-index schemas are an equal prefix of `leaf`'s
            // and it places nothing beyond them, while `leaf` constrains further.
            let looser_items = other.items.is_none()
                && leaf.prefix.starts_with(&other.prefix)
                && (leaf.items.is_some() || leaf.prefix.len() > other.prefix.len());
            let same_elements = other.prefix == leaf.prefix && other.items == leaf.items;
            let wider = other.lengths.covers(&leaf.lengths)
                && (!other.unique || leaf.unique)
                && (looser_items || same_elements);
            // Leaves agreeing on every facet but the window were folded by merging, so one of the
            // facets is strictly looser here and decides which leaf goes.
            debug_assert!(
                !wider || other.unique != leaf.unique || !same_elements,
                "merging left two leaves carrying the same facets"
            );
            if wider && ((leaf.unique && !other.unique) || looser_items) {
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
