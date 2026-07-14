use crate::{
    canonical::{
        context::{CanonicalizationContext, WalkStage},
        intern::shared,
        intersect::multi_type_or_false,
        ir::{
            ArrayLeaf, BoundInteger, IntegerLeaf, NumberLeaf, ObjectLeaf, Schema, SchemaKindSet,
            SharedSchema, StringLeaf,
        },
        leaves::TypedLeaf,
    },
    JsonTypeSet,
};

/// Maps a leaf whose constraints admit no value to `false`, and folds trivial type guards (e.g. a guard with a
/// `true` body accepts everything).
///
/// ```text
/// BEFORE: {"type": "integer", "minimum": 10, "maximum": 5}
/// AFTER:  false
///
/// BEFORE: {"type": "string", "minLength": 5, "maxLength": 3}
/// AFTER:  false
/// ```
#[must_use]
pub(crate) fn normalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<EmptinessStage>(schema, ctx)
}

struct EmptinessStage;

impl super::NormalizeStage for EmptinessStage {
    const WALK: WalkStage = WalkStage::Leaves;
    // No fast-path gate: this stage inspects type-guard/typed-group nodes of every kind, so it must run everywhere.
    const MASK: SchemaKindSet = SchemaKindSet::empty();

    fn rewrite(recursed: SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
        match recursed.as_schema() {
            schema if leaf_is_empty(schema, ctx) => shared(Schema::False),
            Schema::Number(leaf) => match fold_fractional_boundary_exclusion(leaf) {
                Some(folded) => shared(Schema::Number(folded)),
                None => recursed,
            },
            // `TypeGuard(ty, True)` accepts every value: `ty`-instances (body True) and non-`ty` both pass.
            Schema::TypeGuard { body, .. } if matches!(body.as_schema(), Schema::True) => {
                shared(Schema::True)
            }
            // `TypeGuard(ty, False)` rejects every `ty`-instance and accepts the rest: the type-set complement of `ty`.
            Schema::TypeGuard { ty, body } if matches!(body.as_schema(), Schema::False) => {
                let only = JsonTypeSet::from(*ty);
                match Schema::type_set_complement(only) {
                    Some(complement) => multi_type_or_false(complement),
                    None => recursed,
                }
            }
            // `TypedGroup(ty, False)` restricts to `ty`-instances satisfying False - empty.
            Schema::TypedGroup { body, .. } if matches!(body.as_schema(), Schema::False) => {
                shared(Schema::False)
            }
            _ => recursed,
        }
    }
}

/// `true` when `schema` is a leaf whose own facets are provably unsatisfiable — the per-type
/// `Leaf::is_empty` check dispatched in one place rather than per variant.
fn leaf_is_empty(schema: &Schema, ctx: &CanonicalizationContext) -> bool {
    fn check<L: TypedLeaf>(schema: &Schema, ctx: &CanonicalizationContext) -> Option<bool> {
        Some(L::project(schema)?.is_empty(ctx))
    }
    check::<IntegerLeaf>(schema, ctx)
        .or_else(|| check::<NumberLeaf>(schema, ctx))
        .or_else(|| check::<StringLeaf>(schema, ctx))
        .or_else(|| check::<ArrayLeaf>(schema, ctx))
        .or_else(|| check::<ObjectLeaf>(schema, ctx))
        .unwrap_or(false)
}

/// A *fractional* `not multipleOf k` on a bounded interval excluding only the endpoints folds into exclusive
/// bounds (fractional endpoint multiples are never re-provided by an integer sibling, so this is canonical).
/// `None` on an interior multiple, an integer modulus, or anything outside a single bounded exclusion.
fn fold_fractional_boundary_exclusion(leaf: &NumberLeaf) -> Option<NumberLeaf> {
    if leaf.multiple_of.is_some() || leaf.not_multiple_of.len() != 1 {
        return None;
    }
    let modulus = &leaf.not_multiple_of[0];
    if modulus.is_zero() || modulus.denominator_is_one() {
        return None;
    }
    let bounds = &leaf.bounds;
    let (minimum, maximum) = (bounds.minimum.as_ref()?, bounds.maximum.as_ref()?);
    // An undecidable divisibility test (`-> None`) declines the fold like an overflowing span.
    let minimum_hits = modulus.divides(minimum)?;
    let maximum_hits = modulus.divides(maximum)?;
    let lowest = minimum.ceil_div(modulus)?;
    let highest = maximum.floor_div(modulus)?;
    // `highest - lowest + 1`, the count of multiples in `[minimum, maximum]`. An unrepresentable span has
    // interior multiples, so overflow (`-> None`) declines the fold like the `> boundary_hits` case below.
    let closed_count = highest
        .checked_sub(&lowest)
        .and_then(|delta| delta.checked_increment())?;
    let boundary_hits = BoundInteger::from(i64::from(minimum_hits) + i64::from(maximum_hits));
    if closed_count > boundary_hits {
        return None; // a multiple strictly inside the interval
    }
    let mut folded = bounds.clone();
    if minimum_hits {
        folded.exclusive_minimum = true;
    }
    if maximum_hits {
        folded.exclusive_maximum = true;
    }
    Some(NumberLeaf {
        bounds: folded,
        multiple_of: None,
        not_multiple_of: Vec::new(),
    })
}
