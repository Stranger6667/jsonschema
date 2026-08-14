//! Renaming one operand's definition keys so two documents can resolve through one map.
//!
//! A `$defs` key is a name private to the document that declares it. Two versions of one schema
//! routinely give the same key different bodies - editing a shared component is the commonest
//! change there is - and one map cannot bind a name twice. Renaming the keys of one side apart
//! leaves both meanings intact and lets the operation answer.

use std::{collections::BTreeSet, sync::Arc};

use ahash::AHashMap;

use crate::canonical::{
    algebra, emptiness,
    ir::{
        ArrayLeaf, AtLeastTwo, ContainsFacet, NonEmpty, ObjectLeaf, ObjectViolation, PropertyMap,
        Schema, SchemaKind,
    },
    schema::DefinitionMap,
};

/// `definitions` with the renamed entries under their new keys, and every reference redirected.
pub(crate) fn rename_definitions(
    definitions: &DefinitionMap,
    renames: &AHashMap<Arc<str>, Arc<str>>,
) -> DefinitionMap {
    definitions
        .iter()
        .map(|(uri, body)| {
            let uri = renames.get(uri).map_or_else(|| Arc::clone(uri), Arc::clone);
            (uri, rename_references(body, renames))
        })
        .collect()
}

/// `schema` with every reference the table renames pointing at its new name.
pub(crate) fn rename_references(schema: &Schema, renames: &AHashMap<Arc<str>, Arc<str>>) -> Schema {
    match schema.kind() {
        SchemaKind::Reference(uri) => match renames.get(uri) {
            Some(fresh) => Schema::new(SchemaKind::Reference(Arc::clone(fresh))),
            None => schema.clone(),
        },
        SchemaKind::Not(inner) => rebuilt(schema, inner, renames, SchemaKind::Not),
        SchemaKind::AllOf(branches) => {
            rebuilt_branches(schema, branches.as_slice(), renames, SchemaKind::AllOf)
        }
        SchemaKind::AnyOf(branches) => {
            rebuilt_branches(schema, branches.as_slice(), renames, SchemaKind::AnyOf)
        }
        SchemaKind::OneOf(branches) => {
            let mut renamed: Vec<Schema> = branches
                .iter()
                .map(|branch| rename_references(branch, renames))
                .collect();
            if renamed == *branches {
                return schema.clone();
            }
            // Branches are held sorted, and a fresh name orders differently from the one it
            // replaces. Duplicates stay: multiplicity is semantic.
            renamed.sort();
            Schema::new(SchemaKind::OneOf(renamed))
        }
        SchemaKind::Array(leaf) => rename_array(schema, leaf.get(), renames),
        SchemaKind::Object(leaf) => rename_object(schema, leaf.get(), renames),
        // A typed group's body is the value set its constructor packed, which names nothing.
        SchemaKind::TypedGroup { body, .. } => {
            debug_assert!(
                !algebra::contains_reference(body),
                "a typed group's body names a target"
            );
            schema.clone()
        }
        SchemaKind::MultiType(_)
        | SchemaKind::String(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::True
        | SchemaKind::False
        | SchemaKind::Raw(_) => schema.clone(),
    }
}

/// Rebuild a node around one renamed child, or hand the node back where nothing changed.
fn rebuilt(
    schema: &Schema,
    child: &Schema,
    renames: &AHashMap<Arc<str>, Arc<str>>,
    wrap: impl FnOnce(Schema) -> SchemaKind,
) -> Schema {
    let renamed = rename_references(child, renames);
    if renamed == *child {
        return schema.clone();
    }
    Schema::new(wrap(renamed))
}

/// Rebuild a conjunction or a union: renaming reorders the branches, which are held sorted. Two
/// distinct names never become one, so the branches stay distinct and stay at least two.
fn rebuilt_branches(
    schema: &Schema,
    branches: &[Schema],
    renames: &AHashMap<Arc<str>, Arc<str>>,
    wrap: impl FnOnce(AtLeastTwo<Schema>) -> SchemaKind,
) -> Schema {
    let renamed: Vec<Schema> = branches
        .iter()
        .map(|branch| rename_references(branch, renames))
        .collect();
    if renamed == branches {
        return schema.clone();
    }
    let branches =
        AtLeastTwo::new(renamed).expect("renaming is one-to-one, so the branches stay distinct");
    Schema::new(wrap(branches))
}

fn rename_array(
    schema: &Schema,
    leaf: &ArrayLeaf,
    renames: &AHashMap<Arc<str>, Arc<str>>,
) -> Schema {
    let renamed = ArrayLeaf {
        lengths: leaf.lengths.clone(),
        distinctness: leaf.distinctness,
        prefix: leaf
            .prefix
            .iter()
            .map(|schema| rename_references(schema, renames))
            .collect(),
        items: leaf
            .items
            .as_ref()
            .map(|schema| rename_references(schema, renames)),
        // Demands are held sorted by their schema, and a fresh name orders differently from the
        // one it replaces.
        contains: {
            let mut demands: Vec<ContainsFacet> = leaf
                .contains
                .iter()
                .map(|facet| ContainsFacet {
                    schema: rename_references(&facet.schema, renames),
                    minimum: facet.minimum.clone(),
                    maximum: facet.maximum.clone(),
                })
                .collect();
            demands.sort_by(|left, right| left.schema.cmp(&right.schema));
            demands
        },
    };
    if renamed == *leaf {
        return schema.clone();
    }
    Schema::new(SchemaKind::Array(
        NonEmpty::new(renamed).expect("renaming keeps the leaf's window"),
    ))
}

fn rename_object(
    schema: &Schema,
    leaf: &ObjectLeaf,
    renames: &AHashMap<Arc<str>, Arc<str>>,
) -> Schema {
    let entries = |map: &PropertyMap| -> PropertyMap {
        map.iter()
            .map(|(key, schema)| (Arc::clone(key), rename_references(schema, renames)))
            .collect()
    };
    let renamed = ObjectLeaf {
        sizes: leaf.sizes.clone(),
        required: leaf.required.clone(),
        property_names: leaf
            .property_names
            .as_ref()
            .map(|schema| rename_references(schema, renames)),
        properties: entries(&leaf.properties),
        pattern_properties: entries(&leaf.pattern_properties),
        additional: leaf
            .additional
            .as_ref()
            .map(|schema| rename_references(schema, renames)),
        violations: sorted(
            leaf.violations
                .iter()
                .map(|violation| match violation {
                    ObjectViolation::NameFails(schema) => {
                        ObjectViolation::NameFails(rename_references(schema, renames))
                    }
                    ObjectViolation::UndeclaredValueFails {
                        names,
                        patterns,
                        additional,
                    } => ObjectViolation::UndeclaredValueFails {
                        names: names.clone(),
                        patterns: patterns.clone(),
                        additional: rename_references(additional, renames),
                    },
                })
                .collect(),
        ),
    };
    if renamed == *leaf {
        return schema.clone();
    }
    Schema::new(SchemaKind::Object(
        NonEmpty::new(renamed).expect("renaming keeps the leaf's window"),
    ))
}

/// The renames each side takes so one map can bind every key.
///
/// Which side gives way is decided by the two bodies, never by which operand came first: `a op b`
/// and `b op a` must rename the same body or they would answer with different names.
pub(crate) struct Renames {
    pub(crate) left: AHashMap<Arc<str>, Arc<str>>,
    pub(crate) right: AHashMap<Arc<str>, Arc<str>>,
}

impl Renames {
    pub(crate) fn is_empty(&self) -> bool {
        self.left.is_empty() && self.right.is_empty()
    }
}

/// How to bind every key of both maps at once, or `None` where a key naming a resource rather than
/// a private entry disagrees - two documents differing there differ about the resource itself.
pub(crate) fn reconcile(
    left: &DefinitionMap,
    right: &DefinitionMap,
    left_local: &BTreeSet<Arc<str>>,
    right_local: &BTreeSet<Arc<str>>,
) -> Option<Renames> {
    let mut renames = Renames {
        left: AHashMap::new(),
        right: AHashMap::new(),
    };
    let mut used: BTreeSet<Arc<str>> = left.keys().chain(right.keys()).map(Arc::clone).collect();
    let mut pending: Vec<(Arc<str>, bool)> = Vec::new();
    for (uri, ours) in left {
        let Some(theirs) = right.get(uri) else {
            continue;
        };
        if ours == theirs {
            continue;
        }
        if !left_local.contains(uri) || !right_local.contains(uri) {
            return None;
        }
        // The greater body gives way, so the choice is the pair's and not the caller's.
        pending.push((Arc::clone(uri), ours > theirs));
    }
    // Renaming one key changes every body that refers to it, so those keys now differ between the
    // sides too and take fresh names of their own. Following the references back settles which.
    while let Some((uri, on_left)) = pending.pop() {
        // A name the document does not declare privately names a resource, which no side may
        // rename - the same reason a disagreement over one is refused above.
        if !(if on_left { left_local } else { right_local }).contains(&uri) {
            return None;
        }
        let side = if on_left {
            &mut renames.left
        } else {
            &mut renames.right
        };
        if side.contains_key(&uri) {
            continue;
        }
        // One more suffix than there are names in play always leaves one free.
        let fresh = (2..=used.len() + 2)
            .map(|suffix| Arc::<str>::from(format!("{uri}-{suffix}")))
            .find(|candidate| !used.contains(candidate))
            .expect("more suffixes than names to avoid");
        used.insert(Arc::clone(&fresh));
        side.insert(Arc::clone(&uri), fresh);
        let map = if on_left { left } else { right };
        for (holder, body) in map {
            if refers_to(body, &uri) {
                pending.push((Arc::clone(holder), on_left));
            }
        }
    }
    Some(renames)
}

/// Both sides' local keys under the names the renames give them.
pub(crate) fn rename_keys(
    left: &BTreeSet<Arc<str>>,
    left_renames: &AHashMap<Arc<str>, Arc<str>>,
    right: &BTreeSet<Arc<str>>,
    right_renames: &AHashMap<Arc<str>, Arc<str>>,
) -> BTreeSet<Arc<str>> {
    let renamed = |keys: &BTreeSet<Arc<str>>, renames: &AHashMap<Arc<str>, Arc<str>>| {
        keys.iter()
            .map(|uri| renames.get(uri).map_or_else(|| Arc::clone(uri), Arc::clone))
            .collect::<Vec<_>>()
    };
    renamed(left, left_renames)
        .into_iter()
        .chain(renamed(right, right_renames))
        .collect()
}

/// Whether `schema` names `uri` anywhere inside it.
fn refers_to(schema: &Schema, uri: &str) -> bool {
    let mut found = Vec::new();
    emptiness::collect_classified_references(schema, emptiness::Position::InPlace, &mut found);
    found.iter().any(|(named, _)| named.as_ref() == uri)
}

/// Violations are held sorted, and a fresh name orders differently from the one it replaces.
fn sorted(mut violations: Vec<ObjectViolation>) -> Vec<ObjectViolation> {
    violations.sort();
    violations
}
