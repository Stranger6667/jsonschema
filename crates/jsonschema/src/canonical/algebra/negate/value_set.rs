//! Const/enum negation: literal complements and boolean bounds.

use num_traits::One;
use serde_json::Value;

use crate::{
    canonical::{
        intern::shared,
        ir::{
            BooleanBounds, BoundInteger, CanonicalJson, IntegerBounds, IntegerLeaf, Schema,
            SharedSchema,
        },
    },
    JsonType,
};

use super::{any_of_complement, not_wrap};

/// Complement a boolean leaf: every non-boolean kind, plus the opposite boolean value when the leaf was pinned to
/// one (`Any` has no in-kind survivor).
///
/// ```text
/// BEFORE: {"const": true}
/// AFTER:  {"anyOf": [{"type": ["number", "string", "array", "object"]}, {"enum": [false, null]}]}
/// ```
pub(super) fn negate_boolean(bounds: &BooleanBounds) -> SharedSchema {
    match bounds {
        // Open boolean is a saturating value set, handled by the `as_type_set` fast path in `negate`.
        BooleanBounds::Any => unreachable!("Boolean(Any) negated via the as_type_set fast path"),
        BooleanBounds::JustTrue => any_of_complement(
            JsonType::Boolean,
            vec![shared(Schema::Boolean(BooleanBounds::JustFalse))],
        ),
        BooleanBounds::JustFalse => any_of_complement(
            JsonType::Boolean,
            vec![shared(Schema::Boolean(BooleanBounds::JustTrue))],
        ),
    }
}

/// Integer literals get the explicit "less than v or greater than v" expansion; other kinds fall back to
/// `Not(const)`.
///
/// ```text
/// BEFORE: {"const": 5}
/// AFTER:  {"anyOf": [
///           {"type": "integer", "maximum": 4},
///           {"type": "integer", "minimum": 6},
///           {"type": "number", "not": {"type": "number", "multipleOf": 1}},
///           {"type": ["null", "boolean", "string", "array", "object"]}
///         ]}
///
/// BEFORE: {"const": "x"}
/// AFTER:  {"not": {"const": "x"}}
/// ```
pub(super) fn negate_const(value: &CanonicalJson) -> SharedSchema {
    let raw = serde_json::from_str::<Value>(value.as_str())
        .expect("CanonicalJson holds well-formed canonical JSON");
    match &raw {
        Value::Null => any_of_complement(JsonType::Null, Vec::new()),
        Value::Bool(b) => {
            let other_bool = shared(Schema::Boolean(if *b {
                BooleanBounds::JustFalse
            } else {
                BooleanBounds::JustTrue
            }));
            any_of_complement(JsonType::Boolean, vec![other_bool])
        }
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                // The singleton decomposition needs `i - 1` and `i + 1`; at the `i64` extremes
                // those bounds are unrepresentable, so fall back to the exact `not(const)` form.
                if let Some(i) = number.as_i64() {
                    if i != i64::MIN && i != i64::MAX {
                        return negate_integer_singleton(BoundInteger::from(i));
                    }
                }
                return any_of_complement(
                    JsonType::Integer,
                    vec![not_wrap(shared(Schema::Const(value.clone())))],
                );
            }
            any_of_complement(
                JsonType::Number,
                vec![not_wrap(shared(Schema::Const(value.clone())))],
            )
        }
        Value::String(_) => any_of_complement(
            JsonType::String,
            vec![not_wrap(shared(Schema::Const(value.clone())))],
        ),
        Value::Array(_) => any_of_complement(
            JsonType::Array,
            vec![not_wrap(shared(Schema::Const(value.clone())))],
        ),
        Value::Object(_) => any_of_complement(
            JsonType::Object,
            vec![not_wrap(shared(Schema::Const(value.clone())))],
        ),
    }
}

fn negate_integer_singleton(value: BoundInteger) -> SharedSchema {
    let one: BoundInteger = One::one();
    let below = IntegerLeaf {
        bounds: IntegerBounds {
            minimum: None,
            maximum: Some(value.owned() - one.owned()),
            exclusive_minimum: false,
            exclusive_maximum: false,
        },
        multiple_of: None,
        not_multiple_of: Vec::new(),
    };
    let above = IntegerLeaf {
        bounds: IntegerBounds {
            minimum: Some(value + one),
            maximum: None,
            exclusive_minimum: false,
            exclusive_maximum: false,
        },
        multiple_of: None,
        not_multiple_of: Vec::new(),
    };
    any_of_complement(
        JsonType::Integer,
        vec![
            shared(Schema::Integer(below)),
            shared(Schema::Integer(above)),
        ],
    )
}

/// Complement a value set: the conjunction of each member's negation (a value matches iff it differs from every
/// member). Contiguous integer members collapse to the gap-and-tails form.
///
/// ```text
/// BEFORE: {"enum": [1, 2, 3]}
/// AFTER:  {"anyOf": [
///           {"type": "integer", "maximum": 0},
///           {"type": "integer", "minimum": 4},
///           {"type": "number", "not": {"type": "number", "multipleOf": 1}},
///           {"type": ["null", "boolean", "string", "array", "object"]}
///         ]}
/// ```
pub(super) fn negate_enum(values: &[CanonicalJson]) -> SharedSchema {
    if values.is_empty() {
        return shared(Schema::True);
    }
    let conjuncts: Vec<SharedSchema> = values.iter().map(negate_const).collect();
    shared(Schema::AllOf(conjuncts))
}
