//! Folding `allOf` over `$ref` through the schema the reference names.
//!
//! Parsing cannot do this: it runs before the `$defs` bodies are known. The set operations can, so
//! without this pass `canonicalize(doc)` and `intersect` over the same parts give different forms.
//!
//! Unions are left alone (they keep their references), as is any document holding a reference
//! cycle.

use std::{collections::BTreeSet, sync::Arc};

use crate::canonical::{
    algebra,
    context::CanonicalizationContext,
    emptiness,
    ir::{ArrayLeaf, ContainsFacet, ObjectLeaf, ObjectViolation, PropertyMap, Schema, SchemaKind},
    parse::{self, ParseOutput},
    DefinitionMap, ROOT_DEFINITION_KEY,
};

/// Intersections one pass may spend. Past it the document is left as parsed - sound, but it will
/// not match what the set operations produce for the same schema.
const FOLD_BUDGET: u64 = 40_000;

/// Folding rounds one schema may take to settle. A fold can rebuild a leaf through passes that
/// read no targets, leaving conjunctions a resolving read folds again on the next round; a form
/// still moving past the cap is kept as it stands.
const SETTLE_ROUNDS: usize = 8;

/// `parsed` with every `allOf` folded through the schemas its branches reference.
pub(crate) fn through_targets(
    mut parsed: ParseOutput,
    ctx: &CanonicalizationContext,
) -> ParseOutput {
    if !parsed.has_references || !foldable(&parsed) {
        return parsed;
    }
    // Bails on a cycle: a partly-folded cycle is meaningless, and an acyclic map is what lets
    // `algebra::resolved` walk without a visited set. The order puts each body after the bodies it
    // references, so one pass suffices instead of one round per level of the chain.
    let Some(order) = emptiness::settling_order(&parsed.definitions) else {
        return parsed;
    };
    // Normalization must not read targets, or it would produce forms the parse cannot.
    let plain =
        CanonicalizationContext::new(ctx.draft(), ctx.pattern_options(), ctx.validate_formats());
    let mut definitions = parsed.definitions.clone();
    // One context for the whole pass, so the intersection cache and the budget are shared.
    let mut resolving = reading(&definitions, ctx);
    for uri in order {
        let body = definitions
            .get(&uri)
            .cloned()
            .expect("the order names the map's own keys");
        let settled = settle(&body, &definitions, &resolving, &plain);
        // An approximated result is not a canonical form; keep what the parse built. Both contexts
        // answer, since the union folds run against `plain`.
        if approximated(&resolving) || approximated(&plain) {
            return parsed;
        }
        if settled == body {
            continue;
        }
        definitions.insert(uri, settled);
        // Later bodies must see the new one.
        resolving.read_targets(Arc::new(definitions.clone()));
    }
    let root = settle(&parsed.root, &definitions, &resolving, &plain);
    if approximated(&resolving) || approximated(&plain) {
        return parsed;
    }
    parsed.root = root;
    parsed.definitions = definitions;
    // Folding inlines targets, which can leave definitions unreferenced.
    parse::prune_unreachable_definitions(&parsed.root, &mut parsed.definitions);
    parsed
}

/// `folded` to a fixpoint, since one round's rebuilt leaves can hold conjunctions the next
/// round folds.
fn settle(
    schema: &Schema,
    definitions: &DefinitionMap,
    ctx: &CanonicalizationContext,
    plain: &CanonicalizationContext,
) -> Schema {
    let mut current = schema.clone();
    for _ in 0..SETTLE_ROUNDS {
        let next = folded(&current, definitions, ctx, plain);
        if approximated(ctx) || approximated(plain) || next == current {
            return next;
        }
        current = next;
    }
    current
}

/// Whether this run had to approximate anything.
fn approximated(ctx: &CanonicalizationContext) -> bool {
    ctx.saw_inexact_intersection() || ctx.outgrew_distribution()
}

/// A context that resolves references through `definitions`, which hold no cycle.
fn reading(definitions: &DefinitionMap, ctx: &CanonicalizationContext) -> CanonicalizationContext {
    CanonicalizationContext::new(ctx.draft(), ctx.pattern_options(), ctx.validate_formats())
        .within(FOLD_BUDGET)
        .resolving(Arc::new(definitions.clone()), BTreeSet::new())
}

/// Whether this document can be folded at all.
///
/// Two shapes are refused: a `propertyNames`/violation constraint written as a `$ref`, since
/// resolving it can yield something an object leaf cannot hold, and a reference to the document
/// root, which the definition map does not contain.
fn foldable(parsed: &ParseOutput) -> bool {
    parsed
        .definitions
        .values()
        .chain(std::iter::once(&parsed.root))
        .all(settled)
}

fn settled(schema: &Schema) -> bool {
    match schema.kind() {
        SchemaKind::Reference(uri) => uri.as_ref() != ROOT_DEFINITION_KEY,
        SchemaKind::Not(inner) | SchemaKind::TypedGroup { body: inner, .. } => settled(inner),
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => {
            branches.as_slice().iter().all(settled)
        }
        SchemaKind::OneOf(branches) => branches.iter().all(settled),
        SchemaKind::Array(leaf) => {
            let leaf = leaf.get();
            leaf.prefix
                .iter()
                .chain(leaf.items.iter())
                .chain(leaf.contains.iter().map(|facet| &facet.schema))
                .all(settled)
        }
        SchemaKind::Object(leaf) => {
            let leaf = leaf.get();
            leaf.property_names
                .as_ref()
                .is_none_or(|names| !algebra::contains_reference(names))
                && leaf.violations.iter().all(|violation| match violation {
                    ObjectViolation::NameFails(names) => !algebra::contains_reference(names),
                    ObjectViolation::UndeclaredValueFails { additional, .. } => {
                        !algebra::contains_reference(additional)
                    }
                })
                && leaf
                    .properties
                    .values()
                    .chain(leaf.pattern_properties.values())
                    .chain(leaf.additional.iter())
                    .all(settled)
        }
        SchemaKind::MultiType(_)
        | SchemaKind::String(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::True
        | SchemaKind::False
        | SchemaKind::Raw(_) => true,
    }
}

/// `schema` with every `allOf` below it folded.
///
/// `ctx` resolves references - that is what lets a conjunction settle. `plain` does not, and only
/// the union fold uses it: resolving there would replace a branch with the schema it references,
/// which the parse never does.
fn folded(
    schema: &Schema,
    definitions: &DefinitionMap,
    ctx: &CanonicalizationContext,
    plain: &CanonicalizationContext,
) -> Schema {
    match schema.kind() {
        SchemaKind::AllOf(branches) => {
            let conjuncts = each(branches.as_slice(), definitions, ctx, plain);
            // Nothing changed below and no branch resolves: leave it alone.
            if conjuncts == branches.as_slice()
                && !conjuncts.iter().any(|branch| names_a_body(branch, ctx))
            {
                return schema.clone();
            }
            conjuncts
                .into_iter()
                .reduce(|left, right| algebra::intersect(left, right, ctx))
                .unwrap_or_else(|| schema.clone())
        }
        SchemaKind::AnyOf(branches) => {
            let union = each(branches.as_slice(), definitions, ctx, plain);
            if union == branches.as_slice() {
                return schema.clone();
            }
            algebra::union(union, plain)
        }
        SchemaKind::OneOf(branches) => {
            let mut choice = each(branches, definitions, ctx, plain);
            // Once no branch holds a reference, this is the same route the parse takes; skipping it
            // would emit a form that re-parses to something else.
            if !choice.iter().any(algebra::contains_reference) {
                if let Some(expanded) = algebra::concrete_one_of(choice.clone(), definitions, ctx) {
                    return expanded;
                }
            }
            // Asked again now the branches are settled: the fold can prove two of them disjoint
            // where the parse could not, and disjoint branches make `oneOf` an `anyOf`.
            if algebra::choice_folds(&choice, definitions, ctx) {
                return algebra::union(choice, ctx);
            }
            if choice == *branches {
                return schema.clone();
            }
            // A branch matching nothing can never be the one that matches. Duplicates stay: with
            // `oneOf`, two identical branches mean no value can match exactly one.
            choice.retain(|branch| !matches!(branch.kind(), SchemaKind::False));
            choice.sort();
            // Fewer than two left means every pair was disjoint (nothing overlaps an empty branch),
            // so `choice_folds` above already returned the union.
            debug_assert!(
                choice.len() > 1,
                "a choice this thin degrades to a union above"
            );
            Schema::new(SchemaKind::OneOf(choice))
        }
        SchemaKind::Not(inner) => {
            let complemented = folded(inner, definitions, ctx, plain);
            if complemented == *inner {
                return schema.clone();
            }
            Schema::new(SchemaKind::Not(complemented))
        }
        SchemaKind::Array(leaf) => {
            let leaf = leaf.get();
            let items = ArrayLeaf {
                lengths: leaf.lengths.clone(),
                distinctness: leaf.distinctness,
                prefix: each(&leaf.prefix, definitions, ctx, plain),
                items: leaf
                    .items
                    .as_ref()
                    .map(|schema| folded(schema, definitions, ctx, plain)),
                contains: leaf
                    .contains
                    .iter()
                    .map(|facet| ContainsFacet {
                        schema: folded(&facet.schema, definitions, ctx, plain),
                        minimum: facet.minimum.clone(),
                        maximum: facet.maximum.clone(),
                    })
                    .collect(),
            };
            // A pointer in the tail or in a demand bounds the length through its target, which only
            // the resolving run can read.
            let reads_a_body = leaf
                .items
                .as_ref()
                .is_some_and(|tail| names_a_body(tail, ctx))
                || leaf
                    .contains
                    .iter()
                    .any(|facet| names_a_body(&facet.schema, ctx));
            if items == *leaf && !reads_a_body {
                return schema.clone();
            }
            algebra::array_leaf(items, ctx)
        }
        SchemaKind::Object(leaf) => {
            let leaf = leaf.get();
            let entries = |map: &PropertyMap| -> PropertyMap {
                map.iter()
                    .map(|(key, schema)| (Arc::clone(key), folded(schema, definitions, ctx, plain)))
                    .collect()
            };
            let keys = ObjectLeaf {
                sizes: leaf.sizes.clone(),
                required: leaf.required.clone(),
                property_names: leaf.property_names.clone(),
                properties: entries(&leaf.properties),
                pattern_properties: entries(&leaf.pattern_properties),
                additional: leaf
                    .additional
                    .as_ref()
                    .map(|schema| folded(schema, definitions, ctx, plain)),
                violations: leaf.violations.clone(),
            };
            if keys == *leaf {
                return schema.clone();
            }
            algebra::object_leaf(keys, ctx)
        }
        // A reference to an empty schema matches nothing; anything else keeps the reference.
        SchemaKind::Reference(uri) => match ctx.definition(uri) {
            Some(target) if matches!(target.kind(), SchemaKind::False) => Schema::falsy(),
            Some(_) | None => schema.clone(),
        },
        // A typed group holds values, not schemas; neither do the rest.
        SchemaKind::TypedGroup { .. }
        | SchemaKind::MultiType(_)
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

fn each(
    branches: &[Schema],
    definitions: &DefinitionMap,
    ctx: &CanonicalizationContext,
    plain: &CanonicalizationContext,
) -> Vec<Schema> {
    branches
        .iter()
        .map(|branch| folded(branch, definitions, ctx, plain))
        .collect()
}

/// Whether this branch is a reference the run can resolve.
fn names_a_body(schema: &Schema, ctx: &CanonicalizationContext) -> bool {
    matches!(schema.kind(), SchemaKind::Reference(uri) if ctx.definition(uri).is_some())
}
