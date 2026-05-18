//! Classifies each `AllOf` conjunct by JSON type: disjoint kinds short-circuit to `False`, and fully-mergeable
//! conjuncts fold via `intersect_internal`.

use std::sync::Arc;

use crate::{
    canonical::{
        canonicalize_ir,
        context::{CanonicalizationContext, WalkStage},
        intern::shared,
        intersect::{
            has_unmergeable_string_format_pair, has_unmergeable_value_set_pair,
            has_unsafe_numeric_pair, intersect_internal, object::has_unsafe_object_pair,
        },
        ir::{Schema, SchemaKindSet, SharedSchema},
        walk::map_children,
    },
    JsonType,
};

/// Partitions an `allOf` by JSON type: type-disjoint conjuncts are unsatisfiable, while conjuncts that share a type
/// fold into one leaf via `intersect`.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "integer"}, {"type": "string"}]}
/// AFTER:  false
///
/// BEFORE: {"allOf": [{"type": "integer", "minimum": 5},
///                    {"type": "integer", "maximum": 10}]}
/// AFTER:  {"type": "integer", "minimum": 5, "maximum": 10}
/// ```
#[must_use]
pub(crate) fn canonicalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<TypePartitionStage>(schema, ctx)
}

struct TypePartitionStage;

impl super::StructuralStage for TypePartitionStage {
    const WALK: WalkStage = WalkStage::TypePartition;
    // No gate: `AllOf` triggers the fold, but the pass also walks every other shape.
    const MASK: SchemaKindSet = SchemaKindSet::empty();

    fn rewrite(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
        match schema.as_schema() {
            Schema::AllOf(branches) => partition_all_of(schema, branches, ctx),
            _ => map_children(schema, |child| canonicalize(child, ctx)),
        }
    }
}

fn partition_all_of(
    schema: &SharedSchema,
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let recursed: Vec<SharedSchema> = branches
        .iter()
        .map(|branch| canonicalize(branch, ctx))
        .collect();
    let flattened = flatten_all_of(&recursed);

    let mut pinned: Option<JsonType> = None;
    for branch in &flattened {
        let Some(kind) = branch.as_schema().pinned_kind() else {
            continue;
        };
        if let Some(previous) = pinned {
            let Some(narrowed) = previous.intersect(kind) else {
                return shared(Schema::False);
            };
            pinned = Some(narrowed);
        } else {
            pinned = Some(kind);
        }
    }

    // Fold only when every branch is intersect-friendly, else the residual `AllOf` re-enters this stage. Unsafe object
    // pairs (catch-all vs opposite pattern) and numeric pairs (overflowing `multipleOf` LCM) bail to `AllOf`, so folding loops - keep them residual.
    if flattened.iter().all(branch_is_fully_mergeable)
        && !has_unsafe_object_pair(&flattened, ctx)
        && !has_unsafe_numeric_pair(&flattened)
        && !has_unmergeable_string_format_pair(&flattened, ctx)
        && !has_unmergeable_value_set_pair(&flattened, ctx)
    {
        return fold_with_intersect(&flattened, ctx);
    }
    rebuild_all_of(schema, &recursed, branches)
}

/// Shapes `intersect_canonical` always resolves without an `AllOf` fallback. Undecidable `Const`/`Enum` pairings
/// defer and would loop back into this stage; `has_unmergeable_value_set_pair` screens those out.
fn branch_is_fully_mergeable(branch: &SharedSchema) -> bool {
    match branch.as_schema() {
        Schema::Null
        | Schema::Boolean(_)
        | Schema::Integer(_)
        | Schema::Number(_)
        | Schema::String(_)
        | Schema::Array(_)
        | Schema::Object(_)
        | Schema::True
        | Schema::False
        | Schema::Const(_)
        | Schema::Enum(_)
        | Schema::TypedGroup { .. } => true,
        Schema::AnyOf(inner) => inner.iter().all(branch_is_fully_mergeable),
        _ => false,
    }
}

fn flatten_all_of(branches: &[SharedSchema]) -> Vec<SharedSchema> {
    let mut out: Vec<SharedSchema> = Vec::with_capacity(branches.len());
    for branch in branches {
        match branch.as_schema() {
            Schema::AllOf(inner) => out.extend(flatten_all_of(inner)),
            _ => out.push(Arc::clone(branch)),
        }
    }
    out
}

fn fold_with_intersect(branches: &[SharedSchema], ctx: &CanonicalizationContext) -> SharedSchema {
    // Fold with the non-canonicalising intersect, canonicalize once at the end. Intersection is associative and
    // canonicalisation confluent, so the result matches per-step canonicalisation without re-running the pipeline each step.
    let mut iterator = branches.iter();
    let Some(first) = iterator.next() else {
        return shared(Schema::True);
    };
    let mut accumulator = Arc::clone(first);
    for branch in iterator {
        accumulator = intersect_internal(&accumulator, branch, ctx);
        if matches!(accumulator.as_schema(), Schema::False) {
            return accumulator;
        }
    }
    canonicalize_ir(&accumulator, ctx)
}

fn rebuild_all_of(
    schema: &SharedSchema,
    recursed: &[SharedSchema],
    original: &[SharedSchema],
) -> SharedSchema {
    let changed = recursed
        .iter()
        .zip(original.iter())
        .any(|(after, before)| !Arc::ptr_eq(after, before));
    if changed {
        shared(Schema::AllOf(recursed.to_vec()))
    } else {
        Arc::clone(schema)
    }
}
