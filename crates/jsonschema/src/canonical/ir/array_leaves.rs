use crate::canonical::ir::{ArrayLeaf, BoundCardinality, Bounds, LengthBounds};

/// Array leaves merged per uniqueness flag and free of subsumed windows. Inserts are batched; the
/// form is restored before any read, so the order in which leaves arrive cannot change the result.
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
        absorb_trivially_distinct(&mut self.leaves);
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

/// Fold the length windows of leaves that agree on uniqueness.
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
    leaves.sort_by_key(|leaf| leaf.unique);
    let mut merged: Vec<ArrayLeaf> = Vec::with_capacity(leaves.len());
    let mut windows: Vec<LengthBounds> = Vec::new();
    let mut unique: Option<bool> = None;
    for leaf in leaves {
        if unique.is_none_or(|flag| flag != leaf.unique) {
            flush_group(&mut merged, unique.take(), &mut windows);
            unique = Some(leaf.unique);
        }
        windows.push(leaf.lengths);
    }
    flush_group(&mut merged, unique, &mut windows);
    merged
}

/// Emit one leaf per merged window, all carrying the group's uniqueness.
fn flush_group(merged: &mut Vec<ArrayLeaf>, unique: Option<bool>, windows: &mut Vec<LengthBounds>) {
    let Some(unique) = unique else {
        return;
    };
    for window in Bounds::merge_all(std::mem::take(windows)) {
        merged.push(ArrayLeaf {
            lengths: window,
            unique,
        });
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
            && leaf
                .lengths
                .maximum
                .as_ref()
                .is_some_and(|max| *max <= BoundCardinality::from(1))
    }) else {
        return;
    };
    let window = leaves[trivial].lengths.clone();
    // Merging the pair yields one window exactly when they overlap or touch.
    let Some((target, widened)) = leaves.iter().enumerate().find_map(|(index, leaf)| {
        if !leaf.unique {
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
            let wider = other.lengths.covers(&leaf.lengths) && (!other.unique || leaf.unique);
            // Equal flags under `wider` mean merging already folded the two into one leaf, so the
            // flags decide which leaf goes without a tie to break.
            debug_assert!(
                !wider || other.unique != leaf.unique,
                "merging left two leaves agreeing on uniqueness"
            );
            if wider && !other.unique {
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
