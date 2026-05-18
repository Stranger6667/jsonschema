use std::sync::Arc;

use ahash::AHashSet;

use crate::canonical::{
    const_enum::intern_value_set,
    context::{CanonicalizationContext, WalkStage},
    intern::{allof_pair, shared},
    ir::{CanonicalJson, CanonicalKind, OneOf, Schema, SchemaKindSet, SharedSchema},
    walk::map_children,
};

/// Folds a `oneOf` over disjoint value sets into one `enum`; a singleton `oneOf` becomes its branch and an empty one
/// is unsatisfiable. Branches that are not plain value sets keep an explicit mutual-exclusion encoding instead.
///
/// ```text
/// BEFORE: {"oneOf": [{"const": 1}, {"const": 2}, {"enum": [3, 4]}]}
/// AFTER:  {"enum": [1, 2, 3, 4]}
///
/// BEFORE: {"oneOf": [{"type": "string"}]}
/// AFTER:  {"type": "string"}
/// ```
#[must_use]
pub(crate) fn canonicalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<OneOfStage>(schema, ctx)
}

struct OneOfStage;

impl super::StructuralStage for OneOfStage {
    const WALK: WalkStage = WalkStage::OneOf;
    const MASK: SchemaKindSet = SchemaKindSet::of(CanonicalKind::OneOf);

    fn rewrite(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
        canonicalize_impl(schema, ctx)
    }
}

fn canonicalize_impl(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    let Schema::OneOf(OneOf(branches)) = schema.as_schema() else {
        return map_children(schema, |child| canonicalize(child, ctx));
    };
    match branches.as_slice() {
        [] => return shared(Schema::False),
        [only] => return canonicalize(only, ctx),
        _ => {}
    }
    let mut processed: Vec<SharedSchema> = branches.iter().map(|b| canonicalize(b, ctx)).collect();
    // `false` never matches, so it can never be the "exactly one" branch.
    processed.retain(|branch| !matches!(branch.as_schema(), Schema::False));
    match processed.as_slice() {
        [] => return shared(Schema::False),
        [only] => return Arc::clone(only),
        _ => {}
    }

    // Pairwise-disjoint value-set branches need no mutual-exclusion encoding - `oneOf` collapses to the union `Enum`.
    if let Some(merged) = fold_disjoint_value_sets(&processed) {
        return merged;
    }

    linear_encode(&processed)
}

/// `oneOf` over `Const` / `Enum` branches collapses to a single value set when the branches' value sets are pairwise
/// disjoint - mutual exclusion is then automatic and the AnyOf/AllOf/Not scaffolding is unnecessary.
///
/// ```text
/// BEFORE: {"oneOf": [{"const": 1}, {"enum": [2, 3]}]}
/// AFTER:  {"enum": [1, 2, 3]}                            // disjoint values -> one set
///
/// BEFORE: {"oneOf": [{"enum": [1, 2]}, {"enum": [2, 3]}]}
/// AFTER:  None                                           // `2` is shared, so the caller keeps `oneOf`
/// ```
fn fold_disjoint_value_sets(branches: &[SharedSchema]) -> Option<SharedSchema> {
    let mut seen: AHashSet<CanonicalJson> = AHashSet::new();
    let mut all_values: Vec<CanonicalJson> = Vec::new();
    for branch in branches {
        match branch.as_schema() {
            Schema::Const(value) => {
                if !seen.insert(value.clone()) {
                    return None;
                }
                all_values.push(value.clone());
            }
            Schema::Enum(values) => {
                for value in values {
                    if !seen.insert(value.clone()) {
                        return None;
                    }
                    all_values.push(value.clone());
                }
            }
            _ => return None,
        }
    }
    Some(intern_value_set(all_values))
}

/// Encodes "exactly one branch matches" as a balanced reduction tree of O(N) subschemas.
///
/// Each node carries two subschemas over its branches: `O` ("exactly one matches") and `N` ("none match"); a leaf for
/// `b` starts at `O = b`, `N = {"not": b}`. Siblings merge so "exactly one below" holds when one side matches and the other doesn't:
///
/// ```text
/// N_parent = {"allOf": [N_left, N_right]}
/// O_parent = {"anyOf": [{"allOf": [N_left, O_right]},
///                       {"allOf": [O_left, N_right]}]}
/// ```
///
/// Concretely, for two branches `a` and `b` the `O` meaning "exactly one matches" is:
///
/// ```text
/// {"anyOf": [
///   {"allOf": [a, {"not": b}]},   // a matches, b doesn't
///   {"allOf": [{"not": a}, b]}    // b matches, a doesn't
/// ]}
/// ```
///
/// The root's `O` is the whole `oneOf`. Branches are padded to a power of two with `false` leaves (`O = false`,
/// `N = true`) so every level pairs up evenly; padding never matches, so the semantics are unchanged.
fn linear_encode(processed: &[SharedSchema]) -> SharedSchema {
    let real_count = processed.len();
    let padded_size = real_count.next_power_of_two();
    let false_schema = shared(Schema::False);
    let true_schema = shared(Schema::True);

    let mut one_level: Vec<SharedSchema> = Vec::with_capacity(padded_size);
    let mut not_level: Vec<SharedSchema> = Vec::with_capacity(padded_size);
    for branch in processed {
        one_level.push(Arc::clone(branch));
        not_level.push(shared(Schema::Not(Arc::clone(branch))));
    }
    while one_level.len() < padded_size {
        one_level.push(Arc::clone(&false_schema));
        not_level.push(Arc::clone(&true_schema));
    }

    while one_level.len() > 1 {
        let mut next_one: Vec<SharedSchema> = Vec::with_capacity(one_level.len() / 2);
        let mut next_not: Vec<SharedSchema> = Vec::with_capacity(one_level.len() / 2);
        for pair_start in (0..one_level.len()).step_by(2) {
            let left_one = Arc::clone(&one_level[pair_start]);
            let right_one = Arc::clone(&one_level[pair_start + 1]);
            let left_not = Arc::clone(&not_level[pair_start]);
            let right_not = Arc::clone(&not_level[pair_start + 1]);
            next_not.push(allof_pair(&left_not, &right_not));
            next_one.push(shared(Schema::AnyOf(vec![
                allof_pair(&left_not, &right_one),
                allof_pair(&left_one, &right_not),
            ])));
        }
        one_level = next_one;
        not_level = next_not;
    }

    one_level
        .into_iter()
        .next()
        .expect("padded power-of-two reduces to a single root")
}
