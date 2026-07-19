#![cfg_attr(not(feature = "arbitrary-precision"), allow(clippy::clone_on_copy))]

use referencing::Draft;

use crate::{
    canonical::{
        context::keeps_draft4_integer_guard,
        intern::shared,
        intersect::{normalize_integer_not_multiple_of, normalize_number_not_multiple_of},
        ir::{BoundFraction, BoundInteger, IntegerBounds, NumberBounds, Schema, SharedSchema},
    },
    JsonType,
};

/// An integer branch beside `not: {multipleOf: q}` folds each excluded modulus `q` into the positive branch's
/// `not_multiple_of` (same for `number`). A contradiction collapses to `false`.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "integer", "minimum": 0}, {"not": {"type": "integer", "multipleOf": 2}}]}
/// AFTER:  {"type": "integer", "minimum": 0, "not": {"type": "integer", "multipleOf": 2}}
/// ```
pub(crate) fn absorb_not_multiple_of_siblings(
    branches: &mut Vec<SharedSchema>,
    draft: Draft,
) -> bool {
    // `q` if `schema` is an integer leaf with only `multiple_of` set; an open integer leaf is `q = 1`.
    fn integer_multiple_of_only(schema: &Schema) -> Option<BoundInteger> {
        let Schema::Integer(leaf) = schema else {
            return None;
        };
        if leaf.bounds != IntegerBounds::default() || !leaf.not_multiple_of.is_empty() {
            return None;
        }
        (leaf.multiple_of.as_ref())
            .map(BoundInteger::owned)
            .or_else(|| Some(BoundInteger::from(1)))
    }

    // Moduli `q_i` if `schema` is `Not(I)` or `Not(anyOf[I_i])` over leaves `only` accepts. The `anyOf` form is what an
    // emitted `not_multiple_of` re-parses to (`¬(I_2 ∨ I_3) = ¬I_2 ∧ ¬I_3`), so handling it keeps canonicalize a fixpoint.
    fn not_multiple_of_moduli<M>(
        schema: &Schema,
        only: impl Fn(&Schema) -> Option<M>,
    ) -> Option<Vec<M>> {
        let Schema::Not(inner) = schema else {
            return None;
        };
        match inner.as_schema() {
            Schema::AnyOf(branches) => branches
                .iter()
                .map(|branch| only(branch.as_schema()))
                .collect(),
            other => only(other).map(|modulus| vec![modulus]),
        }
    }

    // Fold every `Not(multipleOf)` sibling's moduli into the positive leaf's `not_multiple_of`, only when such a
    // positive sibling exists. `None` signals a contradiction collapsing `branches` to `false`; `Some(changed)` reports rewrite.
    fn absorb<M>(
        branches: &mut Vec<SharedSchema>,
        is_positive: fn(&Schema) -> bool,
        extract_moduli: impl Fn(&Schema) -> Option<Vec<M>>,
        rebuild: fn(&Schema, Vec<M>) -> Option<Schema>,
    ) -> Option<bool> {
        if !branches.iter().any(|b| is_positive(b.as_schema())) {
            return Some(false);
        }
        let mut excluded: Vec<M> = Vec::new();
        branches.retain(|b| match extract_moduli(b.as_schema()) {
            Some(moduli) => {
                excluded.extend(moduli);
                false
            }
            None => true,
        });
        if excluded.is_empty() {
            return Some(false);
        }
        let index = branches
            .iter()
            .position(|b| is_positive(b.as_schema()))
            .expect("positive leaf still present");
        if let Some(schema) = rebuild(branches[index].as_schema(), excluded) {
            branches[index] = shared(schema);
            Some(true)
        } else {
            *branches = vec![shared(Schema::False)];
            None
        }
    }

    // `q` if `schema` is a numeric leaf that only pins `multipleOf` (an open integer leaf is `q = 1`).
    // Under Draft 4 an integer leaf is lexical, not the value-based `multipleOf` spelling, so it
    // contributes no modulus to a `number` branch.
    let numeric_multiple_of_only = |schema: &Schema| -> Option<BoundFraction> {
        match schema {
            Schema::Integer(_) if keeps_draft4_integer_guard(JsonType::Integer, draft) => None,
            Schema::Integer(_) => integer_multiple_of_only(schema).map(BoundFraction::from),
            Schema::Number(leaf)
                if leaf.bounds == NumberBounds::default()
                    && leaf.not_multiple_of.is_empty()
                    && leaf.multiple_of.is_some() =>
            {
                (leaf.multiple_of.as_ref()).map(BoundFraction::owned)
            }
            _ => None,
        }
    };

    let mut changed = false;
    match absorb(
        branches,
        |schema| matches!(schema, Schema::Integer(_)),
        |schema| not_multiple_of_moduli(schema, integer_multiple_of_only),
        |schema, excluded| {
            let Schema::Integer(leaf) = schema else {
                unreachable!("positive integer leaf")
            };
            let mut merged = leaf.clone();
            merged.not_multiple_of.extend(excluded);
            normalize_integer_not_multiple_of(merged).map(Schema::Integer)
        },
    ) {
        None => return true,
        Some(branch_changed) => changed |= branch_changed,
    }
    match absorb(
        branches,
        |schema| matches!(schema, Schema::Number(_)),
        |schema| not_multiple_of_moduli(schema, numeric_multiple_of_only),
        |schema, excluded| {
            let Schema::Number(leaf) = schema else {
                unreachable!("positive number leaf")
            };
            let mut merged = leaf.clone();
            merged.not_multiple_of.extend(excluded);
            normalize_number_not_multiple_of(merged).map(Schema::Number)
        },
    ) {
        None => return true,
        Some(branch_changed) => changed |= branch_changed,
    }
    changed
}
