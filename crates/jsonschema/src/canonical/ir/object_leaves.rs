use std::sync::Arc;

use crate::canonical::ir::{BoundCardinality, Bounds, LengthBounds, ObjectLeaf, Schema};

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
        absorb_trivially_admitted(&mut self.leaves);
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

/// The facets shared by a merge group; only the size window differs within one.
struct Facets {
    required: Vec<Arc<str>>,
    property_names: Option<Schema>,
}

/// Fold the size windows of leaves demanding the same keys under the same key constraint.
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
    leaves.sort_by(|left, right| {
        (&left.required, &left.property_names).cmp(&(&right.required, &right.property_names))
    });
    let mut merged: Vec<ObjectLeaf> = Vec::with_capacity(leaves.len());
    let mut windows: Vec<LengthBounds> = Vec::new();
    let mut facets: Option<Facets> = None;
    for leaf in leaves {
        if facets.as_ref().is_none_or(|group| {
            group.required != leaf.required || group.property_names != leaf.property_names
        }) {
            flush_group(&mut merged, facets.take(), &mut windows);
            facets = Some(Facets {
                required: leaf.required,
                property_names: leaf.property_names,
            });
        }
        windows.push(leaf.sizes);
    }
    flush_group(&mut merged, facets, &mut windows);
    merged
}

/// Emit one leaf per merged window, cloning the keys onto each and moving them into the last.
fn flush_group(
    merged: &mut Vec<ObjectLeaf>,
    facets: Option<Facets>,
    windows: &mut Vec<LengthBounds>,
) {
    let Some(Facets {
        required,
        property_names,
    }) = facets
    else {
        return;
    };
    let mut sizes = Bounds::merge_all(std::mem::take(windows));
    let last = sizes.pop().expect("a group holds at least one window");
    for window in sizes {
        merged.push(ObjectLeaf {
            sizes: window,
            required: required.clone(),
            property_names: property_names.clone(),
        });
    }
    merged.push(ObjectLeaf {
        sizes: last,
        required,
        property_names,
    });
}

/// A window of no properties holds only the empty object, whose keys satisfy any constraint
/// vacuously: widen a neighbouring key-constrained leaf over it. Only a leaf demanding no key can
/// absorb it, since a required key rejects the empty object whatever the window says.
/// ```text
/// e.g.  anyOf [
///         {"const": {}},
///         {"type": "object", "propertyNames": {"const": "a"}, "minProperties": 1}
///       ]  =>  {"type": "object", "propertyNames": {"const": "a"}}
/// ```
///
/// A gap between the two windows keeps them apart: widening would admit the property counts
/// between them.
/// ```text
/// e.g.  anyOf [
///         {"const": {}},
///         {"type": "object", "propertyNames": {"enum": ["a", "b"]}, "minProperties": 2}
///       ]  =>  unchanged
/// ```
fn absorb_trivially_admitted(leaves: &mut Vec<ObjectLeaf>) {
    let Some(trivial) = leaves.iter().position(|leaf| {
        leaf.property_names.is_none()
            && leaf.required.is_empty()
            && leaf
                .sizes
                .maximum
                .as_ref()
                .is_some_and(BoundCardinality::is_zero)
    }) else {
        return;
    };
    let window = leaves[trivial].sizes.clone();
    // Merging the pair yields one window exactly when they overlap or touch. Leaves sharing the
    // absent key constraint sit in the trivial leaf's own merge group, so only a key-constrained
    // leaf can be the target.
    let Some((target, widened)) = leaves.iter().enumerate().find_map(|(index, leaf)| {
        if leaf.property_names.is_none() || !leaf.required.is_empty() {
            return None;
        }
        let mut merged = Bounds::merge_all(vec![leaf.sizes.clone(), window.clone()]);
        (merged.len() == 1).then(|| (index, merged.pop().expect("a merged window")))
    }) else {
        return;
    };
    leaves[target].sizes = widened;
    leaves.remove(trivial);
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
            // A key constraint is compared only against itself: deciding that one schema admits
            // every key another does needs a subsumption test the algebra does not have. The
            // windows compare with a finite set of admitted keys folded in, since such a leaf
            // admits no object larger than that set.
            let looser_keys = other.property_names.is_none() && leaf.property_names.is_some();
            let wider = other.effective_sizes().covers(&leaf.effective_sizes())
                && other.required.iter().all(|key| leaf.required.contains(key))
                && (looser_keys || other.property_names == leaf.property_names);
            // Leaves agreeing on every facet but the window were folded by merging, so one of the
            // facets is strictly looser here and decides which leaf goes.
            debug_assert!(
                !wider
                    || other.required.len() != leaf.required.len()
                    || other.property_names != leaf.property_names,
                "merging left two leaves carrying the same facets"
            );
            if wider && (other.required.len() < leaf.required.len() || looser_keys) {
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
