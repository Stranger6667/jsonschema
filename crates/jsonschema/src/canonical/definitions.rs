//! The `$ref` / `$defs` graph: the shared definition-body map and the operations that keep it sound.
//!
//! A [`CanonicalSchema`](super::CanonicalSchema) carries a transitive-closure [`DefinitionMap`] from every reachable
//! symbolic ref to its target. The algebra combines two such maps, sound only when their keyspaces agree:
//!
//! - [`union_definitions`] merges compatible maps (left wins on a shared key).
//! - [`disambiguate_definitions`] relocates *both* operands into disjoint keyspaces when a merge could rebind a ref,
//!   keeping the result operand-order-independent.
//! - [`reachable_definitions`] prunes unreferenced entries, upholding the minimality invariant that keeps results idempotent.

use std::{cmp::Ordering, collections::BTreeMap, sync::Arc};

use ahash::{AHashMap, AHashSet};

use crate::canonical::{
    emit,
    intern::shared,
    ir::{CanonicalKind, Schema, SchemaKindSet, SharedSchema},
    walk,
};

/// Reference uri -> canonical target body.
pub(crate) type DefinitionMap = BTreeMap<Arc<str>, SharedSchema>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
enum DefinitionSide {
    Left,
    Right,
}

/// (Operand side, definition uri) -> fresh collision-free name, for relocating definitions before a union.
type DefinitionRenames = AHashMap<(DefinitionSide, Arc<str>), Arc<str>>;

/// Union of two definition maps, sharing an `Arc` when one side is empty. On a key collision the left operand wins, so
/// callers must ensure the operands never disagree on a shared key (see [`disambiguate_definitions`]).
pub(crate) fn union_definitions(
    left: &Arc<DefinitionMap>,
    right: &Arc<DefinitionMap>,
) -> Arc<DefinitionMap> {
    if right.is_empty() {
        return Arc::clone(left);
    }
    if left.is_empty() {
        return Arc::clone(right);
    }
    let mut merged = (**left).clone();
    for (uri, body) in right.iter() {
        match merged.entry(Arc::clone(uri)) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(Arc::clone(body));
            }
            // Differing bodies on a collision mean a caller skipped `disambiguate_definitions`, silently
            // rebinding the right side's refs to the left's body.
            std::collections::btree_map::Entry::Occupied(existing) => debug_assert!(
                existing.get() == body,
                "union_definitions: key `{uri}` bound to differing bodies; disambiguate before union"
            ),
        }
    }
    Arc::new(merged)
}

/// Move both operands' definitions into disjoint keyspaces when merging could rebind a symbolic ref.
///
/// *All* keys are renamed, not just colliding ones: an identical-looking body can still alias the other side through a
/// differing transitive ref. Names derive from sorted operand-side entries, so renames stay order-independent. Compatible maps pass untouched.
pub(crate) fn disambiguate_definitions(
    left_inner: &SharedSchema,
    left: &Arc<DefinitionMap>,
    right_inner: &SharedSchema,
    right: &Arc<DefinitionMap>,
) -> (
    (SharedSchema, Arc<DefinitionMap>),
    (SharedSchema, Arc<DefinitionMap>),
) {
    let Some(rename) = symmetric_renames(left_inner, left, right_inner, right) else {
        return (
            (Arc::clone(left_inner), Arc::clone(left)),
            (Arc::clone(right_inner), Arc::clone(right)),
        );
    };
    let relocate_side = |inner: &SharedSchema, side_name: DefinitionSide, side: &DefinitionMap| {
        let rename_for_side: AHashMap<Arc<str>, Arc<str>> = side
            .keys()
            .map(|key| {
                let new_key = rename
                    .get(&(side_name, Arc::clone(key)))
                    .expect("every entry was named");
                (Arc::clone(key), Arc::clone(new_key))
            })
            .collect();
        let inner = relocate_refs(inner, &rename_for_side);
        let mut relocated = DefinitionMap::new();
        for (key, body) in side {
            let new_key = rename_for_side
                .get(key)
                .map_or_else(|| Arc::clone(key), Arc::clone);
            relocated.insert(new_key, relocate_refs(body, &rename_for_side));
        }
        (inner, Arc::new(relocated))
    };
    (
        relocate_side(left_inner, DefinitionSide::Left, left),
        relocate_side(right_inner, DefinitionSide::Right, right),
    )
}

/// Fresh names for every operand-side entry when the maps are unsafe to merge; `None` when compatible.
///
/// Assigned over entries sorted by key, body, and source map, so both operand orders produce identical names without
/// coalescing equal-looking bodies that may resolve through different local definitions.
fn symmetric_renames(
    left_inner: &SharedSchema,
    left: &DefinitionMap,
    right_inner: &SharedSchema,
    right: &DefinitionMap,
) -> Option<DefinitionRenames> {
    let unsafe_to_merge = definitions_disagree(left, right)
        || dangling_refs_overlap_definitions(left_inner, left, right)
        || dangling_refs_overlap_definitions(right_inner, right, left);
    if !unsafe_to_merge {
        return None;
    }
    let mut entries: Vec<(DefinitionSide, &Arc<str>, &SharedSchema)> = left
        .iter()
        .map(|(key, body)| (DefinitionSide::Left, key, body))
        .chain(
            right
                .iter()
                .map(|(key, body)| (DefinitionSide::Right, key, body)),
        )
        .collect();
    // Compare source maps once; the per-entry tiebreak maps each side through this ordering, staying order-independent without re-comparing whole maps in the sort.
    let source_order = left.cmp(right);
    entries.sort_by(|(side_a, key_a, body_a), (side_b, key_b, body_b)| {
        key_a
            .cmp(key_b)
            .then_with(|| body_a.cmp(body_b))
            // `Left` < `Right` by declaration; flip when the right map sorts first so the tiebreak stays
            // operand-order-independent. Same-side pairs never reach here (a `BTreeMap` has unique keys).
            .then_with(|| {
                let by_side = side_a.cmp(side_b);
                if source_order == Ordering::Greater {
                    by_side.reverse()
                } else {
                    by_side
                }
            })
    });
    let mut used: AHashSet<Arc<str>> = AHashSet::new();
    reserve_symbolic_ref_uris(left_inner, left, &mut used);
    reserve_symbolic_ref_uris(right_inner, right, &mut used);
    let mut rename = AHashMap::with_capacity(entries.len());
    let mut counter = 0_usize;
    for (side_name, key, _) in entries {
        let leaf = key
            .rsplit('/')
            .next()
            .filter(|segment| !segment.is_empty())
            .unwrap_or("root");
        let candidate = loop {
            let candidate: Arc<str> = format!("#/$defs/{leaf}__merge{counter}").into();
            counter += 1;
            if !used.contains(&candidate) {
                break candidate;
            }
        };
        used.insert(Arc::clone(&candidate));
        rename.insert((side_name, Arc::clone(key)), candidate);
    }
    Some(rename)
}

fn reserve_symbolic_ref_uris(
    inner: &SharedSchema,
    definitions: &DefinitionMap,
    used: &mut AHashSet<Arc<str>>,
) {
    used.extend(
        collect_all_symbolic_refs(inner, definitions)
            .into_iter()
            .map(|uri| Arc::from(emit::strip_synthetic_root(uri.as_ref()))),
    );
}

fn definitions_disagree(left: &DefinitionMap, right: &DefinitionMap) -> bool {
    !left.is_empty()
        && right
            .iter()
            .any(|(key, body)| left.get(key).is_some_and(|existing| existing != body))
}

fn dangling_refs_overlap_definitions(
    inner: &SharedSchema,
    own_definitions: &DefinitionMap,
    other_definitions: &DefinitionMap,
) -> bool {
    if other_definitions.is_empty() {
        return false;
    }
    collect_all_symbolic_refs(inner, own_definitions)
        .into_iter()
        .any(|uri| {
            definition_entry(own_definitions, uri.as_ref()).is_none()
                && definition_entry(other_definitions, uri.as_ref()).is_some()
        })
}

/// Rebuild `node`, rewriting every `Reference`/`Recursive` uri found in `rename`. Subtrees that carry no symbolic
/// reference are returned shared, so the common case allocates nothing.
fn relocate_refs(node: &SharedSchema, rename: &AHashMap<Arc<str>, Arc<str>>) -> SharedSchema {
    const SYMBOLIC: SchemaKindSet =
        SchemaKindSet::from_kinds(&[CanonicalKind::Reference, CanonicalKind::Recursive]);
    if node.mask.is_disjoint(SYMBOLIC) {
        return Arc::clone(node);
    }
    match node.as_schema() {
        // Ref uris may carry the synthetic `json-schema:///` root while keys are stripped; normalize before lookup, like `definition_entry`.
        Schema::Reference(uri) => relocate_ref(node, uri.as_str(), rename, |target| {
            let target = referencing::uri::from_str(target)
                .expect("renamed key is a valid same-document pointer");
            Schema::Reference(target)
        }),
        Schema::Recursive(uri) => relocate_ref(node, uri, rename, |target| {
            Schema::Recursive(Arc::clone(target))
        }),
        _ => walk::map_children(node, |child| relocate_refs(child, rename)),
    }
}

/// Look up a symbolic ref's (synthetic-root-normalized) uri in `rename`; rebuild the node under the fresh
/// name via `build`, or keep it shared when the uri is not relocated (a ref dangling past the renamed keys).
fn relocate_ref(
    node: &SharedSchema,
    uri: &str,
    rename: &AHashMap<Arc<str>, Arc<str>>,
    build: impl FnOnce(&Arc<str>) -> Schema,
) -> SharedSchema {
    match rename.get(emit::strip_synthetic_root(uri)) {
        Some(target) => shared(build(target)),
        None => Arc::clone(node),
    }
}

pub(crate) fn reachable_definitions(
    root: &SharedSchema,
    definitions: &Arc<DefinitionMap>,
) -> Arc<DefinitionMap> {
    if definitions.is_empty() {
        return Arc::clone(definitions);
    }

    let mut reachable = DefinitionMap::new();
    let mut visited: AHashSet<*const ()> = AHashSet::new();
    let mut pending = Vec::new();
    collect_symbolic_refs(root, &mut visited, &mut pending);

    while let Some(uri) = pending.pop() {
        let Some((key, body)) = definition_entry(definitions, uri.as_ref()) else {
            continue;
        };
        if reachable.contains_key(key) {
            continue;
        }
        reachable.insert(Arc::clone(key), Arc::clone(body));
        collect_symbolic_refs(body, &mut visited, &mut pending);
    }

    Arc::new(reachable)
}

pub(crate) fn definition_entry<'a>(
    definitions: &'a DefinitionMap,
    uri: &str,
) -> Option<(&'a Arc<str>, &'a SharedSchema)> {
    definitions
        .get_key_value(uri)
        .or_else(|| definitions.get_key_value(emit::strip_synthetic_root(uri)))
}

/// Every symbolic ref uri reachable from `inner` and the bodies in `definitions`, walked once per distinct node.
pub(crate) fn collect_all_symbolic_refs(
    inner: &SharedSchema,
    definitions: &DefinitionMap,
) -> Vec<Arc<str>> {
    let mut visited = AHashSet::new();
    let mut refs = Vec::new();
    collect_symbolic_refs(inner, &mut visited, &mut refs);
    for body in definitions.values() {
        collect_symbolic_refs(body, &mut visited, &mut refs);
    }
    refs
}

fn collect_symbolic_refs(
    node: &SharedSchema,
    visited: &mut AHashSet<*const ()>,
    out: &mut Vec<Arc<str>>,
) {
    // The IR is an Arc-shared DAG with exponentially many paths to a shared subtree; memoize by node identity to keep the walk linear in distinct nodes.
    if !visited.insert(Arc::as_ptr(node).cast::<()>()) {
        return;
    }
    match node.as_schema() {
        Schema::Reference(uri) => out.push(Arc::from(uri.as_str())),
        Schema::Recursive(uri) => out.push(Arc::clone(uri)),
        _ => node.for_each_child(|child| collect_symbolic_refs(child, visited, out)),
    }
}
