use std::sync::Arc;

use crate::canonical::ir::{Bounds, LengthBounds, ObjectLeaf};

/// Object leaves merged per required-key set and free of subsumed windows. Inserts are batched; the
/// form is restored before any read, so the order in which leaves arrive cannot change the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectLeaves {
    leaves: Vec<ObjectLeaf>,
    canonical: bool,
}

impl Default for ObjectLeaves {
    fn default() -> Self {
        Self {
            leaves: Vec::new(),
            canonical: true,
        }
    }
}

impl ObjectLeaves {
    pub(crate) fn insert(&mut self, leaf: ObjectLeaf) {
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
    pub(crate) fn retain(&mut self, keep: impl FnMut(&ObjectLeaf) -> bool) {
        self.canonicalize();
        self.leaves.retain(keep);
    }

    /// Merging never removes the last leaf, so this reads the batch without canonicalizing.
    pub(crate) fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub(crate) fn as_slice(&mut self) -> &[ObjectLeaf] {
        self.canonicalize();
        &self.leaves
    }
}

impl IntoIterator for ObjectLeaves {
    type Item = ObjectLeaf;
    type IntoIter = std::vec::IntoIter<ObjectLeaf>;

    fn into_iter(mut self) -> Self::IntoIter {
        self.canonicalize();
        self.leaves.into_iter()
    }
}

/// Fold the size windows of leaves demanding the same keys.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "required": ["a"], "maxProperties": 2},
///         {"type": "object", "required": ["a"], "minProperties": 3}
///       ]  =>  {"type": "object", "required": ["a"]}
/// ```
///
/// Different keys admit different objects, so those leaves stay apart.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "required": ["a"]},
///         {"type": "object", "required": ["b"]}
///       ]  =>  unchanged
/// ```
fn merge(mut leaves: Vec<ObjectLeaf>) -> Vec<ObjectLeaf> {
    if leaves.len() < 2 {
        return leaves;
    }
    leaves.sort_by(|left, right| left.required.cmp(&right.required));
    let mut merged: Vec<ObjectLeaf> = Vec::with_capacity(leaves.len());
    let mut windows: Vec<LengthBounds> = Vec::new();
    let mut required: Option<Vec<Arc<str>>> = None;
    for leaf in leaves {
        if required.as_ref().is_none_or(|keys| *keys != leaf.required) {
            flush_group(&mut merged, required.take(), &mut windows);
            required = Some(leaf.required);
        }
        windows.push(leaf.sizes);
    }
    flush_group(&mut merged, required, &mut windows);
    merged
}

/// Emit one leaf per merged window, cloning the keys onto each and moving them into the last.
fn flush_group(
    merged: &mut Vec<ObjectLeaf>,
    required: Option<Vec<Arc<str>>>,
    windows: &mut Vec<LengthBounds>,
) {
    let Some(required) = required else {
        return;
    };
    let mut sizes = Bounds::merge_all(std::mem::take(windows));
    let last = sizes.pop().expect("a group holds at least one window");
    for window in sizes {
        merged.push(ObjectLeaf {
            sizes: window,
            required: required.clone(),
        });
    }
    merged.push(ObjectLeaf {
        sizes: last,
        required,
    });
}

/// Drop a leaf whose objects another leaf already admits: a window that contains it, demanding no
/// key it does not. Extra keys only narrow, so the leaf demanding fewer swallows the other.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "required": ["a"]},
///         {"type": "object", "required": ["a", "b"]}
///       ]  =>  {"type": "object", "required": ["a"]}
/// ```
fn drop_subsumed(leaves: &mut Vec<ObjectLeaf>) {
    if leaves.len() < 2 {
        return;
    }
    let mut keep = vec![true; leaves.len()];
    for (index, leaf) in leaves.iter().enumerate() {
        for (other_index, other) in leaves.iter().enumerate() {
            if index == other_index || !keep[other_index] || !keep[index] {
                continue;
            }
            let wider = other.sizes.covers(&leaf.sizes)
                && other.required.iter().all(|key| leaf.required.contains(key));
            // Equal key counts under `wider` mean equal key sets, and merging already folded those
            // into one leaf, so the counts decide which leaf goes without a tie to break.
            debug_assert!(
                !wider || other.required.len() != leaf.required.len(),
                "merging left two leaves demanding the same keys"
            );
            if wider && other.required.len() < leaf.required.len() {
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
