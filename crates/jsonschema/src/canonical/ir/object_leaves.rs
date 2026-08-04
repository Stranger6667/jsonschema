use std::sync::Arc;

use std::collections::BTreeMap;

use crate::canonical::ir::{BoundCardinality, Bounds, LengthBounds, ObjectLeaf, Schema};

/// Object leaves merged per required-key set. Inserts are batched; the form is restored before any
/// read, so the order in which leaves arrive cannot change the result.
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
        extend_over_bare_windows(&mut self.leaves);
        hand_off_empty(&mut self.leaves);
        // Extending can overlap the windows of leaves sharing their facets; fold those again.
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

    /// Merging never removes the last leaf, so this reads the batch without canonicalizing.
    pub(crate) fn is_empty(&self) -> bool {
        self.leaves.is_empty()
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
    properties: BTreeMap<Arc<str>, Schema>,
    pattern_properties: BTreeMap<Arc<str>, Schema>,
    additional: Option<Schema>,
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
        (
            &left.required,
            &left.property_names,
            &left.properties,
            &left.pattern_properties,
            &left.additional,
        )
            .cmp(&(
                &right.required,
                &right.property_names,
                &right.properties,
                &right.pattern_properties,
                &right.additional,
            ))
    });
    let mut merged: Vec<ObjectLeaf> = Vec::with_capacity(leaves.len());
    let mut windows: Vec<LengthBounds> = Vec::new();
    let mut facets: Option<Facets> = None;
    for leaf in leaves {
        if facets.as_ref().is_none_or(|group| {
            group.required != leaf.required
                || group.property_names != leaf.property_names
                || group.properties != leaf.properties
                || group.pattern_properties != leaf.pattern_properties
                || group.additional != leaf.additional
        }) {
            flush_group(&mut merged, facets.take(), &mut windows);
            facets = Some(Facets {
                required: leaf.required,
                property_names: leaf.property_names,
                properties: leaf.properties,
                pattern_properties: leaf.pattern_properties,
                additional: leaf.additional,
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
        properties,
        pattern_properties,
        additional,
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
            properties: properties.clone(),
            pattern_properties: pattern_properties.clone(),
            additional: additional.clone(),
        });
    }
    merged.push(ObjectLeaf {
        sizes: last,
        required,
        property_names,
        properties,
        pattern_properties,
        additional,
    });
}

/// Widen a facet-carrying window over a bare sibling window it touches: the sizes gained lie
/// inside the bare window, which admits those objects with any content, so the union is unchanged.
/// The boundary between the two then has one spelling, whatever the facet leaf's window said.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "maxProperties": 1},
///         {"type": "object", "properties": {"a": {"type": "integer"}}, "minProperties": 2}
///       ]  =>  anyOf [
///         {"type": "object", "maxProperties": 1},
///         {"type": "object", "properties": {"a": {"type": "integer"}}}
///       ]
/// ```
fn extend_over_bare_windows(leaves: &mut [ObjectLeaf]) {
    let bare: Vec<LengthBounds> = leaves
        .iter()
        .filter(|leaf| {
            leaf.required.is_empty()
                && leaf.property_names.is_none()
                && leaf.properties.is_empty()
                && leaf.pattern_properties.is_empty()
                && leaf.additional.is_none()
        })
        .map(|leaf| leaf.sizes.clone())
        .collect();
    if bare.is_empty() {
        return;
    }
    for leaf in leaves.iter_mut() {
        if leaf.required.is_empty()
            && leaf.property_names.is_none()
            && leaf.properties.is_empty()
            && leaf.pattern_properties.is_empty()
            && leaf.additional.is_none()
        {
            continue;
        }
        // A grown window can reach the next bare window, so retry until none applies.
        loop {
            let mut grown = false;
            for window in &bare {
                let merged = Bounds::merge_all(vec![leaf.sizes.clone(), window.clone()]);
                if let Ok([merged]) = <[_; 1]>::try_from(merged) {
                    if merged != leaf.sizes {
                        leaf.sizes = merged;
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

/// Drop a `minProperties: 1` when another branch admits the empty object: the drop adds only `{}`,
/// which that branch accepts and whose keys satisfy any constraint vacuously. A leaf demanding a
/// key never carries that minimum, since it folds into the required count.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": {"type": "integer"}}},
///         {"type": "object", "propertyNames": {"maxLength": 3}, "minProperties": 1}
///       ]  =>  anyOf [
///         {"type": "object", "properties": {"a": {"type": "integer"}}},
///         {"type": "object", "propertyNames": {"maxLength": 3}}
///       ]
/// ```
fn hand_off_empty(leaves: &mut [ObjectLeaf]) {
    if !leaves.iter().any(|leaf| {
        leaf.required.is_empty()
            && leaf
                .sizes
                .minimum
                .as_ref()
                .is_none_or(BoundCardinality::is_zero)
    }) {
        return;
    }
    let one = BoundCardinality::from(1);
    for leaf in leaves.iter_mut() {
        if leaf.sizes.minimum.as_ref() == Some(&one) {
            leaf.sizes.minimum = None;
        }
    }
}
