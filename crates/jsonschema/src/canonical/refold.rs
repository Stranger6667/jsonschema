//! Folding `allOf` over `$ref` through the schema the reference names.
//!
//! Parsing cannot do this: it runs before the `$defs` bodies are known. The set operations can, so
//! without this pass `canonicalize(doc)` and `intersect` over the same parts give different forms.
//!
//! Unions keep their references; a document holding a reference cycle is left as parsed.

use std::{collections::BTreeSet, sync::Arc};

use crate::canonical::{
    algebra,
    context::CanonicalizationContext,
    emptiness,
    ir::{ArrayLeaf, ContainsFacet, ObjectLeaf, ObjectViolation, PropertyMap, Schema, SchemaKind},
    negate,
    parse::{self, ParseOutput},
    DefinitionMap, ROOT_DEFINITION_KEY,
};

/// Intersections one pass may spend. Past it the document is left as parsed - sound, but it will
/// not match what the set operations produce for the same schema.
const FOLD_BUDGET: u64 = 40_000;

/// Folding rounds one schema may take to settle. A round can rebuild a leaf without reading
/// targets, leaving `allOf`s the next round folds; past the cap the last round's form stands.
const SETTLE_ROUNDS: usize = 8;

/// `parsed` with every `allOf` folded through the schemas its branches reference.
pub(crate) fn through_targets(
    mut parsed: ParseOutput,
    ctx: &CanonicalizationContext,
) -> ParseOutput {
    let Some(settled) = Settled::of(&parsed, ctx) else {
        return parsed;
    };
    let Some(root) = settled.settle(&parsed.root) else {
        return parsed;
    };
    parsed.root = root;
    parsed.definitions = settled.into_definitions();
    // Folding inlines targets, which can leave definitions unreferenced.
    parse::prune_unreachable_definitions(&parsed.root, &mut parsed.definitions);
    parsed
}

/// A document's bodies folded through what they reference, for settling the schemas read
/// against them the same way.
pub(crate) struct Settled {
    /// One context for the whole pass, so the intersection cache and the budget are shared. It
    /// owns its copy of the map, so a settled body lands in place and later bodies read it.
    resolving: CanonicalizationContext,
    /// Normalization must not read targets, or it would produce forms the parse cannot.
    plain: CanonicalizationContext,
}

impl Settled {
    /// `None` where the document cannot be folded: it holds no reference, a shape `foldable`
    /// refuses, a reference cycle, or a body the pass could only approximate.
    pub(crate) fn of(parsed: &ParseOutput, ctx: &CanonicalizationContext) -> Option<Self> {
        if !parsed.has_references || !foldable(parsed) {
            return None;
        }
        // Bails on a cycle: a partly-folded cycle is meaningless, and an acyclic map is what lets
        // `algebra::resolved` walk without a visited set. The order puts each body after the
        // bodies it references, so one pass suffices instead of one round per level of the chain.
        let order = emptiness::settling_order(&parsed.definitions)?;
        let plain = CanonicalizationContext::new(
            ctx.draft(),
            ctx.pattern_options(),
            ctx.validate_formats(),
        );
        let mut resolving = reading(&parsed.definitions, ctx);
        for uri in order {
            let body = resolving
                .targets()
                .get(&uri)
                .cloned()
                .expect("the order names the map's own keys");
            let settled = settle(&body, resolving.targets(), &resolving, &plain);
            // An approximated result is not a canonical form; keep what the parse built. Both
            // contexts answer, since the union folds run against `plain`.
            if approximated(&resolving) || approximated(&plain) {
                return None;
            }
            if settled == body {
                continue;
            }
            resolving.targets_mut().insert(uri, settled);
        }
        Some(Self { resolving, plain })
    }

    /// `schema` folded through the settled bodies, or `None` where the pass could only
    /// approximate it.
    pub(crate) fn settle(&self, schema: &Schema) -> Option<Schema> {
        let settled = settle(
            schema,
            self.resolving.targets(),
            &self.resolving,
            &self.plain,
        );
        (!approximated(&self.resolving) && !approximated(&self.plain)).then_some(settled)
    }

    pub(crate) fn definitions(&self) -> &DefinitionMap {
        self.resolving.targets()
    }

    /// The context reading through the settled bodies.
    pub(crate) fn context(&self) -> &CanonicalizationContext {
        &self.resolving
    }

    fn into_definitions(self) -> DefinitionMap {
        self.resolving.into_targets()
    }
}

/// `folded` until it stops changing, since one round's rebuilt leaves can hold `allOf`s the next
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
/// Two shapes are refused: a `propertyNames` or violation schema holding a `$ref`, since
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
                    ObjectViolation::PatternValueFails { schema, .. } => {
                        !algebra::contains_reference(schema)
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
/// `ctx` resolves references - that is what lets an `allOf` settle. `plain` does not; the union
/// and negation folds run against it, since a resolving union would replace a branch with the
/// schema it references, which the parse never does.
fn folded(
    schema: &Schema,
    definitions: &DefinitionMap,
    ctx: &CanonicalizationContext,
    plain: &CanonicalizationContext,
) -> Schema {
    match schema.kind() {
        SchemaKind::AllOf(branches) => {
            let folded = each(branches.as_slice(), definitions, ctx, plain);
            // Nothing changed below and no branch resolves: leave it alone.
            if folded == branches.as_slice()
                && !folded.iter().any(|branch| names_a_body(branch, ctx))
            {
                return schema.clone();
            }
            folded
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
            let choice = each(branches, definitions, ctx, plain);
            // The parse's own route: settling can leave branches repeated or disjoint where the
            // parse saw neither, and the output must reduce as its re-parse would.
            algebra::one_of(
                choice,
                definitions,
                &DefinitionMap::new(),
                &mut Vec::new(),
                ctx,
            )
            .unwrap_or_else(|| schema.clone())
        }
        SchemaKind::Not(inner) => {
            let body = folded(inner, definitions, ctx, plain);
            if body == *inner {
                return schema.clone();
            }
            // The folded body may be one negation expresses, or nothing at all: wrapping it raw
            // would leave a `not` the parse never builds, such as `not: false`. Where negation
            // still declines, the node stands unfolded.
            negate::negate_in_place(&body, definitions, plain).unwrap_or_else(|| schema.clone())
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
                PropertyMap::from_sorted(
                    map.iter()
                        .map(|(key, schema)| {
                            (Arc::clone(key), folded(schema, definitions, ctx, plain))
                        })
                        .collect(),
                )
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
        // A reference to `false` matches nothing; anything else keeps the reference.
        SchemaKind::Reference(uri) => match ctx.definition(uri) {
            Some(target) if matches!(target.kind(), SchemaKind::False) => Schema::falsy(),
            Some(_) | None => schema.clone(),
        },
        // A typed group's body is a value set; the rest hold no subschemas.
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

/// Whether this branch is a reference the run can resolve, or a `oneOf` over one.
fn names_a_body(schema: &Schema, ctx: &CanonicalizationContext) -> bool {
    if let SchemaKind::OneOf(branches) = schema.kind() {
        return branches.iter().any(|branch| names_a_body(branch, ctx));
    }
    matches!(schema.kind(), SchemaKind::Reference(uri) if ctx.definition(uri).is_some())
}
