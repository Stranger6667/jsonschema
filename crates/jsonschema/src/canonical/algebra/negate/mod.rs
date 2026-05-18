//! Algebraic negation of canonical IR schemas.
//!
//! Output is the exact complement but not canonical; callers re-run the pipeline on it. Four output
//! shapes escape the canonical fragment and block structural `negate(negate(x)) == x`: symbolic
//! `Not(...)` residuals, same-kind unions from overlapping branches, `AnyOf`-of-`AllOf`
//! reconstructions of `oneOf`/`if`-`then`-`else`, and opaque nodes (`$ref`/`Raw`).

mod array;
mod numeric;
mod object;
mod string;
mod value_set;

use std::sync::Arc;

use referencing::Draft;

use crate::{
    canonical::{
        context::{CanonicalizationContext, WalkStage},
        intern::shared,
        intersect::{multi_type_or_false, open_typed_leaf},
        ir::{IfThenElse, OneOf, Schema, SharedSchema},
    },
    JsonType, JsonTypeSet,
};

use self::{
    array::{negate_array_leaf, negate_array_unique_items_type_guard},
    numeric::{
        negate_number_multiple_of_type_guard, negate_numeric, negate_numeric_bounds_type_guard,
    },
    object::{negate_object_leaf, negate_object_required_type_guard},
    string::{negate_string, negate_string_pattern_type_guard},
    value_set::{negate_boolean, negate_const, negate_enum},
};

/// Complement the schema's value set. Returns a non-canonicalized result so pipeline callers can re-enter it
/// safely. Booleans distribute via De Morgan; a typed leaf negates to every other type (`integer ⊂ number`, so
/// the `number` slot keeps only fractional values) unioned with the leaf's own bound complement.
///
/// ```text
/// BEFORE: {"allOf": [A, B]}
/// AFTER:  {"anyOf": [{"not": A}, {"not": B}]}
///
/// BEFORE: {"type": "integer", "minimum": 5, "maximum": 10}
/// AFTER:  {"anyOf": [
///           {"type": ["null", "boolean", "string", "array", "object"]},
///           {"allOf": [{"type": "number"}, {"not": {"type": "integer"}}]},
///           {"type": "integer", "maximum": 4},
///           {"type": "integer", "minimum": 11}
///         ]}
/// ```
#[must_use]
pub(crate) fn negate(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    negate_with_options(
        schema,
        NegationOptions {
            use_contains_for_array_tail: true,
        },
        ctx,
    )
}

#[must_use]
pub(crate) fn negate_for_draft(
    schema: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    negate_with_options(
        schema,
        NegationOptions {
            use_contains_for_array_tail: ctx.draft().is_known_keyword("contains"),
        },
        ctx,
    )
}

#[derive(Clone, Copy)]
struct NegationOptions {
    use_contains_for_array_tail: bool,
}

fn negate_with_options(
    schema: &SharedSchema,
    options: NegationOptions,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    // Memoize by node identity: a shared subgraph negates identically along every path, so an inlined DAG isn't
    // re-negated per edge (exponential otherwise). Result depends on `use_contains_for_array_tail`, so the two
    // settings key into separate stages.
    let stage = if options.use_contains_for_array_tail {
        WalkStage::Negate
    } else {
        WalkStage::NegateWithoutContains
    };
    ctx.with_walk_memo(stage, schema, || negate_inner(schema, options, ctx))
}

fn negate_inner(
    schema: &SharedSchema,
    options: NegationOptions,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    // Saturating value sets (open typed leaves, `Const(null)`, `Enum([false, true])`, `MultiType`) negate via
    // type-set complement: skips the per-variant paths and keeps the result minimal.
    if let Some(set) = schema.as_schema().as_type_set() {
        return negate_multi_type(set);
    }
    match schema.as_schema() {
        Schema::True => shared(Schema::False),
        Schema::False => shared(Schema::True),
        Schema::AllOf(branches) => {
            let negated: Vec<SharedSchema> = branches
                .iter()
                .map(|branch| negate_with_options(branch, options, ctx))
                .collect();
            shared(Schema::AnyOf(negated))
        }
        Schema::AnyOf(branches) => {
            let negated: Vec<SharedSchema> = branches
                .iter()
                .map(|branch| negate_with_options(branch, options, ctx))
                .collect();
            shared(Schema::AllOf(negated))
        }
        Schema::OneOf(OneOf(branches)) => negate_one_of(branches, options, ctx),
        Schema::Not(inner) => Arc::clone(inner),
        Schema::IfThenElse(node) => negate_if_then_else(node, options, ctx),

        // Open `null` is a saturating value set, handled by the `as_type_set` fast path above.
        Schema::Null => unreachable!("Null negated via the as_type_set fast path"),
        Schema::Boolean(bounds) => negate_boolean(bounds),
        Schema::Integer(leaf) => negate_numeric(leaf),
        Schema::Number(leaf) => negate_numeric(leaf),
        Schema::String(leaf) => negate_string(leaf),
        Schema::Object(leaf) => negate_object_leaf(schema, leaf, options, ctx),
        Schema::Array(leaf) => negate_array_leaf(schema, leaf, options, ctx),

        Schema::TypedGroup { ty: kind, body } => negate_typed_group(*kind, body, options, ctx),
        Schema::TypeGuard { ty: kind, body } => negate_type_guard(*kind, body, ctx.draft())
            .unwrap_or_else(|| {
                shared(Schema::TypedGroup {
                    ty: *kind,
                    body: negate_with_options(body, options, ctx),
                })
            }),

        Schema::Const(value) => negate_const(value, ctx.draft()),
        Schema::Enum(values) => negate_enum(values, ctx.draft()),

        Schema::Reference(_) | Schema::Recursive(_) | Schema::DynamicRef(_) | Schema::Raw(_) => {
            shared(Schema::Not(Arc::clone(schema)))
        }
        // A `MultiType` is a type set, handled by the `as_type_set` fast path above.
        Schema::MultiType(_) => unreachable!("MultiType negated via the as_type_set fast path"),
    }
}

/// Complement: a value matches iff its type is NOT in `set`. Falls back to `Not(MultiType(...))` when the complement
/// isn't expressible as a type set.
fn negate_multi_type(set: JsonTypeSet) -> SharedSchema {
    match Schema::type_set_complement(set) {
        Some(complement) => multi_type_or_false(complement),
        // Collapse the inner so a single-element set is canonical as `Not(Integer)`, never `Not(MultiType({Integer}))`.
        None => not_wrap(multi_type_or_false(set)),
    }
}

fn not_wrap(inner: SharedSchema) -> SharedSchema {
    shared(Schema::Not(inner))
}

/// Negate a single-facet type guard whose body has an IR-direct dual, keeping the type pin: `pattern` ↔
/// `not_patterns`, `uniqueItems` ↔ `repeated_items`, a single `required` name ↔ that property forbidden.
/// Returns `None` for any other body (caller wraps the guard in `Not`); used by `not_elim`.
pub(crate) fn negate_type_guard(
    kind: JsonType,
    body: &SharedSchema,
    draft: Draft,
) -> Option<SharedSchema> {
    negate_string_pattern_type_guard(kind, body)
        .or_else(|| negate_array_unique_items_type_guard(kind, body))
        .or_else(|| negate_object_required_type_guard(kind, body))
        .or_else(|| negate_number_multiple_of_type_guard(kind, body, draft))
        .or_else(|| negate_numeric_bounds_type_guard(kind, body, draft))
}

/// Wrap in-kind negation branches into the single `AnyOf` entry `any_of_complement` expects, or nothing when empty.
fn wrap_in_kind(branches: Vec<SharedSchema>) -> Vec<SharedSchema> {
    // Empty when every facet negated to nothing in-kind (e.g. an all-`True` prefix): the complement is then
    // just the other JSON types, with no in-kind `AnyOf` entry.
    if branches.is_empty() {
        Vec::new()
    } else {
        vec![shared(Schema::AnyOf(branches))]
    }
}

/// `AnyOf` of "every kind but `exclude`" plus any in-kind branches. Integer is a subtype of Number, so excluding
/// Integer restricts Number to fractional values and excluding Number drops Integer transitively.
fn any_of_complement(exclude: JsonType, in_kind: Vec<SharedSchema>) -> SharedSchema {
    let mut branches: Vec<SharedSchema> = Vec::new();
    for kind in JsonTypeSet::all() {
        if kind == exclude {
            continue;
        }
        match (exclude, kind) {
            (JsonType::Integer, JsonType::Number) => {
                branches.push(shared(Schema::AllOf(vec![
                    open_typed_leaf(JsonType::Number),
                    not_wrap(open_typed_leaf(JsonType::Integer)),
                ])));
            }
            (JsonType::Number, JsonType::Integer) => {}
            _ => branches.push(open_typed_leaf(kind)),
        }
    }
    branches.extend(in_kind);
    shared(Schema::AnyOf(branches))
}

fn negate_typed_group(
    kind: JsonType,
    body: &SharedSchema,
    options: NegationOptions,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    // The body is always a constraining leaf/value-set (no producer builds `TypedGroup{ty, True}`), so the
    // in-kind survivor is the type-pinned negation of that body.
    any_of_complement(
        kind,
        vec![shared(Schema::TypedGroup {
            ty: kind,
            body: negate_with_options(body, options, ctx),
        })],
    )
}

/// "Exactly one" complements to "none, or two or more": `AllOf(not b_i)` for the none case, plus a pairwise
/// `AllOf(b_i, b_j)` per overlap (disjoint branches make every pair empty, so only the none case survives).
///
/// ```text
/// BEFORE: {"oneOf": [{"type": "string"}, {"type": "object"}]}
/// AFTER:  {"type": ["null", "boolean", "number", "array"]}
/// ```
fn negate_one_of(
    branches: &[SharedSchema],
    options: NegationOptions,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let none_match: Vec<SharedSchema> = branches
        .iter()
        .map(|branch| negate_with_options(branch, options, ctx))
        .collect();
    let count = branches.len();
    // One `AllOf` for the none case, plus one per unordered pair of branches.
    let mut combined: Vec<SharedSchema> =
        Vec::with_capacity(1 + count * count.saturating_sub(1) / 2);
    combined.push(shared(Schema::AllOf(none_match)));
    for index in 0..branches.len() {
        for other in (index + 1)..branches.len() {
            combined.push(shared(Schema::AllOf(vec![
                Arc::clone(&branches[index]),
                Arc::clone(&branches[other]),
            ])));
        }
    }
    shared(Schema::AnyOf(combined))
}

/// Complement the desugared `anyOf(allOf(if, then), allOf(not if, else))`: a value must fail both arms, i.e. match
/// `if` but violate `then`, or violate `if` and `else`.
///
/// ```text
/// BEFORE: {"if": {"type": "integer"}, "then": {"minimum": 0}, "else": {"type": "string"}}
/// AFTER:  {"anyOf": [
///           {"type": "integer", "maximum": -1},
///           {"type": "number", "not": {"type": "number", "multipleOf": 1}},
///           {"type": ["null", "boolean", "array", "object"]}
///         ]}
/// ```
fn negate_if_then_else(
    node: &IfThenElse,
    options: NegationOptions,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let condition = Arc::clone(&node.condition);
    let neg_condition = negate_with_options(&condition, options, ctx);
    let then_branch = node
        .then_branch
        .clone()
        .unwrap_or_else(|| shared(Schema::True));
    let else_branch = node
        .else_branch
        .clone()
        .unwrap_or_else(|| shared(Schema::True));
    let neg_then = negate_with_options(&then_branch, options, ctx);
    let neg_else = negate_with_options(&else_branch, options, ctx);
    shared(Schema::AnyOf(vec![
        shared(Schema::AllOf(vec![condition, neg_then])),
        shared(Schema::AllOf(vec![neg_condition, neg_else])),
    ]))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::needless_pass_by_value)]

    // Each row supplies witness sets (accepts / rejects of the original schema).

    use referencing::Draft;
    use serde_json::{json, Value};
    use test_case::test_case;

    use crate::{
        canonical::{
            intern::shared,
            ir::{BooleanBounds, IfThenElse, IntegerLeaf, OneOf, Schema, StringLeaf},
            tests_util::{assert_json_complement, canonicalize, cardinality, const_json},
            CanonicalSchema,
        },
        JsonType,
    };

    fn negated_value(schema: &Value) -> Value {
        canonicalize(schema).negate().to_json_schema()
    }

    // `true` when the IR has no obvious redundant structure: empty/singleton compositions, `AllOf` with `True`,
    // `AnyOf` with `False`, `Not(Not(_))`, or a `TypedGroup` that should have unwrapped to its body.
    fn has_no_obvious_redundant_structure(schema: &Schema) -> bool {
        let local_ok = match schema {
            Schema::AllOf(branches) | Schema::AnyOf(branches) | Schema::OneOf(OneOf(branches)) => {
                if branches.len() < 2 {
                    return false;
                }
                let absorber = match schema {
                    Schema::AllOf(_) => Some(Schema::True),
                    Schema::AnyOf(_) => Some(Schema::False),
                    _ => None,
                };
                !absorber.is_some_and(|absorber| {
                    branches
                        .iter()
                        .any(|branch| branch.as_schema() == &absorber)
                })
            }
            Schema::Not(inner) => !matches!(inner.as_schema(), Schema::Not(_)),
            Schema::TypedGroup { ty, body } => !body.as_schema().is_typed_leaf_of(*ty),
            _ => true,
        };
        if !local_ok {
            return false;
        }
        schema
            .children()
            .iter()
            .all(|child| has_no_obvious_redundant_structure(child.as_schema()))
    }

    // Negate a directly-constructed IR variant and assert exact set complement (rejects the originals's accepts,
    // accepts its rejects). Reaches per-variant `negate` arms the pipeline rewrites away before `negate` runs.
    fn assert_ir_complement(schema: Schema, accepts: &[Value], rejects: &[Value]) {
        let node = CanonicalSchema::from_inner(shared(schema), Draft::Draft202012);
        let positive = node.to_json_schema();
        let negated = node.negate().to_json_schema();
        let pos = crate::validator_for(&positive).expect("positive compiles");
        let neg = crate::validator_for(&negated).expect("negated compiles");
        for value in accepts {
            assert!(pos.is_valid(value));
            assert!(!neg.is_valid(value));
        }
        for value in rejects {
            assert!(!pos.is_valid(value));
            assert!(neg.is_valid(value));
        }
    }

    #[test_case(json!({"type": "integer", "multipleOf": 3}),
            &[json!(0), json!(3), json!(-6)],
            &[json!(1), json!(2), json!("x"), json!(1.5)]
            ; "integer_multiple_of")]
    #[test_case(json!({"type": "string", "pattern": "^a"}),
            &[json!("abc"), json!("aaa")],
            &[json!("xyz"), json!(0), json!(null)]
            ; "string_pattern")]
    #[test_case(json!({"type": "array", "uniqueItems": true}),
            &[json!([]), json!([1, 2, 3]), json!([1, 2])],
            &[json!([1, 1]), json!([1, 2, 1]), json!(0), json!(null)]
            ; "array_unique")]
    fn negate_leaf_is_closed(schema: Value, accepts: &[Value], rejects: &[Value]) {
        assert_json_complement(&schema, accepts, rejects);
    }

    // `subtract(self)` = S intersect not-S is empty; `is_satisfiable` must prove it for each closed facet.
    #[test_case(json!({"type": "object", "additionalProperties": {"type": "string"}}) ; "additional_properties existential vs universal")]
    #[test_case(json!({"const": 5}) ; "const vs type-set")]
    #[test_case(json!({"enum": [1, 2, 3]}) ; "enum vs type-set")]
    #[test_case(json!({"type": "array", "items": {"type": "integer"}}) ; "items vs contains")]
    #[test_case(json!({"type": "object", "properties": {"a": {"type": "integer"}}, "additionalProperties": false}) ; "named property vs existential")]
    fn subtract_self_is_unsatisfiable(schema: Value) {
        let s = canonicalize(&schema);
        assert!(!s.subtract(&s).is_satisfiable());
    }

    fn typed_group_string_enum() -> Schema {
        Schema::TypedGroup {
            ty: JsonType::String,
            body: shared(Schema::Enum(vec![
                const_json(json!("a")),
                const_json(json!("b")),
            ])),
        }
    }

    fn type_guard_string_minlen() -> Schema {
        Schema::TypeGuard {
            ty: JsonType::String,
            body: shared(Schema::String(StringLeaf {
                min_length: Some(cardinality(2)),
                ..StringLeaf::default()
            })),
        }
    }

    fn one_of_integer_string() -> Schema {
        Schema::OneOf(OneOf(vec![
            shared(Schema::Integer(IntegerLeaf::default())),
            shared(Schema::String(StringLeaf::default())),
        ]))
    }

    fn if_then_else_ir() -> Schema {
        Schema::IfThenElse(IfThenElse {
            condition: shared(Schema::Integer(IntegerLeaf::default())),
            then_branch: Some(shared(Schema::Integer(IntegerLeaf::default()))),
            else_branch: Some(shared(Schema::String(StringLeaf::default()))),
        })
    }

    fn if_then_only_ir() -> Schema {
        Schema::IfThenElse(IfThenElse {
            condition: shared(Schema::Integer(IntegerLeaf::default())),
            then_branch: None,
            else_branch: Some(shared(Schema::String(StringLeaf::default()))),
        })
    }

    fn if_else_only_ir() -> Schema {
        Schema::IfThenElse(IfThenElse {
            condition: shared(Schema::Integer(IntegerLeaf::default())),
            then_branch: Some(shared(Schema::Integer(IntegerLeaf::default()))),
            else_branch: None,
        })
    }

    // `negate()` must return a canonical schema with no redundant structure, stable under re-canonicalization.
    // Regression guard for when the negation result was not fed back through normalization.
    #[test_case(json!({"type": "string", "minLength": 3}) ; "string min length")]
    #[test_case(json!({"type": "integer", "minimum": 0, "maximum": 10}) ; "bounded integer")]
    #[test_case(json!({"not": {"type": "integer"}}) ; "not integer")]
    #[test_case(json!({"allOf": [{"type": "integer"}, {"minimum": 0}]}) ; "all_of")]
    #[test_case(json!({"oneOf": [{"type": "integer"}, {"type": "string"}]}) ; "one_of")]
    #[test_case(json!({"type": "array", "uniqueItems": true}) ; "unique array")]
    #[test_case(json!({"pattern": "^a"}) ; "untyped pattern")]
    fn negate_returns_canonical_form(schema: Value) {
        let negated = canonicalize(&schema).negate();
        assert!(has_no_obvious_redundant_structure(negated.as_schema()));
        let reparsed = canonicalize(&negated.to_json_schema());
        assert_eq!(negated.as_schema(), reparsed.as_schema());
    }

    #[test_case(json!(false) => false ; "false schema")]
    #[test_case(json!(true) => true ; "true schema")]
    #[test_case(json!({}) => true ; "empty schema")]
    #[test_case(json!({"type": "integer", "minimum": 5, "maximum": 3}) => false ; "empty integer interval")]
    #[test_case(json!({"type": "string", "minLength": 5, "maxLength": 2}) => false ; "empty string interval")]
    #[test_case(json!({"enum": []}) => false ; "empty enum")]
    #[test_case(json!({"allOf": [{"type": "string"}, {"type": "integer"}]}) => false ; "conflicting types")]
    #[test_case(json!({"type": "integer"}) => true ; "integer")]
    #[test_case(json!({"const": 1}) => true ; "const_value")]
    fn is_satisfiable_reports_provable_emptiness(schema: Value) -> bool {
        canonicalize(&schema).is_satisfiable()
    }

    #[test_case(json!(true),
            &[json!(null), json!(0), json!("x"), json!([]), json!({})],
            &[]
            ; "true_schema")]
    #[test_case(json!(false),
            &[],
            &[json!(null), json!(0), json!("x"), json!([]), json!({})]
            ; "false_schema")]
    #[test_case(json!({"type": "null"}),
            &[json!(null)],
            &[json!(0), json!(false), json!("x"), json!([]), json!({})]
            ; "null_type")]
    #[test_case(json!({"type": "integer"}),
            &[json!(0), json!(-5), json!(42)],
            &[json!(0.5), json!("x"), json!(null), json!(true), json!([]), json!({})]
            ; "integer_type")]
    #[test_case(json!({"type": "number"}),
            &[json!(0), json!(-5.5), json!(42)],
            &[json!("x"), json!(null), json!(true), json!([]), json!({})]
            ; "number_type")]
    #[test_case(json!({"type": "string"}),
            &[json!(""), json!("x"), json!("hello")],
            &[json!(0), json!(null), json!(true), json!([]), json!({})]
            ; "string_type")]
    #[test_case(json!({"type": "boolean"}),
            &[json!(true), json!(false)],
            &[json!(0), json!(null), json!("x"), json!([]), json!({})]
            ; "boolean_type")]
    #[test_case(json!({"type": "array"}),
            &[json!([]), json!([1, 2])],
            &[json!(0), json!(null), json!(true), json!("x"), json!({})]
            ; "array_type")]
    #[test_case(json!({"type": "object"}),
            &[json!({}), json!({"a": 1})],
            &[json!(0), json!(null), json!(true), json!("x"), json!([])]
            ; "object_type")]
    #[test_case(json!({"const": 42}),
            &[json!(42)],
            &[json!(0), json!(41), json!(43), json!("42"), json!(null), json!(true)]
            ; "const_integer")]
    #[test_case(json!({"const": null}),
            &[json!(null)],
            &[json!(0), json!(false), json!("null"), json!([])]
            ; "const_null")]
    #[test_case(json!({"const": "abc"}),
            &[json!("abc")],
            &[json!("abcd"), json!("ab"), json!(0), json!(null)]
            ; "const_string")]
    #[test_case(json!({"const": true}),
            &[json!(true)],
            &[json!(false), json!(0), json!(1), json!("true"), json!(null)]
            ; "const_bool_true")]
    #[test_case(json!({"enum": [1, 2, 3]}),
            &[json!(1), json!(2), json!(3)],
            &[json!(0), json!(4), json!("1"), json!(null)]
            ; "enum_ints")]
    #[test_case(json!({"enum": [1, "abc", null]}),
            &[json!(1), json!("abc"), json!(null)],
            &[json!(0), json!("a"), json!(false), json!([])]
            ; "enum_mixed")]
    #[test_case(json!({"type": "integer", "minimum": 0, "maximum": 10}),
            &[json!(0), json!(5), json!(10)],
            &[json!(-1), json!(11), json!(5.5), json!("x"), json!(null)]
            ; "integer_interval")]
    #[test_case(json!({"type": "number", "minimum": 1.5, "exclusiveMaximum": 2.5}),
            &[json!(1.5), json!(2.0), json!(2.49)],
            &[json!(1.0), json!(2.5), json!(3.0), json!("x"), json!(null)]
            ; "number_excl_max")]
    #[test_case(json!({"type": "string", "minLength": 2, "maxLength": 4}),
            &[json!("ab"), json!("abcd")],
            &[json!(""), json!("a"), json!("hello"), json!(0), json!(null)]
            ; "string_length_interval")]
    #[test_case(json!({"allOf": [{"type": "integer"}, {"minimum": 0}]}),
            &[json!(0), json!(5)],
            &[json!(-1), json!(0.5), json!("x"), json!(null)]
            ; "all_of_int_min")]
    #[test_case(json!({"anyOf": [{"type": "integer"}, {"type": "string"}]}),
            &[json!(0), json!("x")],
            &[json!(0.5), json!(null), json!([]), json!({}), json!(true)]
            ; "any_of_int_or_string")]
    #[test_case(json!({"not": {"type": "string"}}),
            &[json!(0), json!(null), json!([]), json!({}), json!(true)],
            &[json!(""), json!("x"), json!("hello")]
            ; "not_string_double_negates")]
    #[test_case(json!({"if": {"type": "integer"}, "then": {"minimum": 0}, "else": {"type": "string"}}),
            &[json!(0), json!(5), json!("x")],
            &[json!(-1), json!(null), json!([]), json!(true), json!(0.5)]
            ; "if_then_else")]
    #[test_case(json!({"oneOf": [{"type": "integer"}, {"type": "string"}]}),
            &[json!(0), json!("x")],
            &[json!(0.5), json!(null), json!([]), json!({})]
            ; "one_of_disjoint")]
    #[test_case(json!({"oneOf": [{"type": "number"}, {"type": "integer"}]}),
            &[json!(0.5), json!(-1.5)],
            &[json!(5), json!("x"), json!(null), json!(true)]
            ; "one_of_overlap")]
    #[test_case(json!({"type": "object", "required": ["a"]}),
            &[json!({"a": 1}), json!({"a": null, "b": 2})],
            &[json!({}), json!({"b": 1}), json!(0), json!("x"), json!(null)]
            ; "object_required_field")]
    #[test_case(json!({"type": "array", "items": {"type": "integer"}}),
            &[json!([1, 2]), json!([]), json!([0])],
            &[json!([1, "x"]), json!(0), json!("x"), json!(null)]
            ; "array_int_items")]
    #[test_case(json!({"type": "integer", "multipleOf": 3}),
            &[json!(0), json!(-3), json!(9)],
            &[json!(1), json!(2.5), json!("x"), json!(null)]
            ; "integer_multiple_of")]
    #[test_case(json!({"not": {"const": 5}}),
            &[json!(0), json!(4), json!("5"), json!(null)],
            &[json!(5)]
            ; "not_const_double_negates")]
    #[test_case(json!({"allOf": [{"type": "string", "minLength": 2}, {"not": {"const": "ab"}}]}),
            &[json!("xy"), json!("hello")],
            &[json!(""), json!("a"), json!("ab"), json!(0), json!(null)]
            ; "string_minus_one_value")]
    #[test_case(json!({"type": "integer", "exclusiveMinimum": 0, "exclusiveMaximum": 5}),
            &[json!(1), json!(4)],
            &[json!(0), json!(5), json!(-1), json!("x"), json!(null)]
            ; "integer_strict_interval")]
    #[test_case(json!({"type": "string", "pattern": "^a"}),
            &[json!("apple"), json!("a")],
            &[json!("banana"), json!(""), json!(0), json!(null)]
            ; "string_pattern_uses_not_wrapper")]
    #[test_case(json!({"type": "array", "uniqueItems": true, "minItems": 2}),
            &[json!([1, 2]), json!(["a", "b", "c"])],
            &[json!([]), json!([1]), json!([1, 1]), json!([1, 2, 1]), json!("x"), json!(null)]
            ; "array_unique_items_with_length_decomposes")]
    #[test_case(json!({"type": "array", "uniqueItems": true, "items": {"type": "integer"}}),
            &[json!([1, 2]), json!([]), json!([0])],
            &[json!([1, "x"]), json!([1, 1]), json!([2, 2, 3]), json!(0), json!("x")]
            ; "array_unique_items_with_items_decomposes")]
    #[test_case(json!({"type": "object", "properties": {"a": {"type": "integer"}}}),
            &[json!({}), json!({"a": 1}), json!({"a": 1, "b": "x"})],
            &[json!({"a": "not-int"}), json!({"a": null, "b": 5}), json!(0), json!("x")]
            ; "object_property_type_uses_pattern_required")]
    #[test_case(json!({"type": "object", "minProperties": 2}),
            &[json!({"a": 1, "b": 2}), json!({"a": 1, "b": 2, "c": 3})],
            &[json!({}), json!({"a": 1}), json!(0), json!("x"), json!(null)]
            ; "object_min_properties_flips_to_max")]
    #[test_case(json!({"type": "object", "maxProperties": 1}),
            &[json!({}), json!({"a": 1})],
            &[json!({"a": 1, "b": 2}), json!({"a": 1, "b": 2, "c": 3})]
            ; "object_max_properties_flips_to_min")]
    #[test_case(json!({"type": "object", "patternProperties": {"^x": {"type": "integer"}}}),
            &[json!({}), json!({"x1": 1}), json!({"y": "anything"})],
            &[json!({"x1": "not-int"}), json!({"y": 1, "x2": "bad"})]
            ; "object_pattern_constraint_uses_pattern_required")]
    #[test_case(json!({"type": "array", "minItems": 2}),
            &[json!([1, 2]), json!([1, 2, 3])],
            &[json!([]), json!([1]), json!(0), json!("x"), json!(null)]
            ; "array_min_items_flips_to_max")]
    #[test_case(json!({"type": "array", "maxItems": 2}),
            &[json!([]), json!([1]), json!([1, 2])],
            &[json!([1, 2, 3]), json!([1, 2, 3, 4])]
            ; "array_max_items_flips_to_min")]
    #[test_case(json!({"type": "array", "prefixItems": [{"type": "integer"}]}),
            &[json!([]), json!([1]), json!([1, "anything"]), json!([1, 2, 3])],
            &[json!(["not-int"]), json!([null, "x"]), json!([true, 0])]
            ; "array_prefix_position_uses_negated_prefix")]
    #[test_case(json!({"type": "array", "items": {"type": "integer"}}),
            &[json!([]), json!([1, 2, 3])],
            &[json!(["x"]), json!([1, "x", 2]), json!([null])]
            ; "array_tail_negation_uses_contains")]
    #[test_case(json!({"type": "number", "multipleOf": 0.5}),
            &[json!(0), json!(0.5), json!(-1.5)],
            &[json!(0.25), json!("x"), json!(null)]
            ; "number_multiple_of_isolated_in_not")]
    #[test_case(json!({"type": "string", "format": "email"}),
            &[json!("x"), json!("anything")],
            &[json!(0), json!(null), json!(true)]
            ; "string_format_uses_not_wrapper")]
    #[test_case(json!({"const": 1.5}),
            &[json!(1.5)],
            &[json!(1), json!(2), json!("1.5"), json!(null)]
            ; "const_fractional_number")]
    #[test_case(json!({"const": [1, 2]}),
            &[json!([1, 2])],
            &[json!([1]), json!([1, 2, 3]), json!(0), json!(null)]
            ; "const_array")]
    #[test_case(json!({"const": {"a": 1}}),
            &[json!({"a": 1})],
            &[json!({}), json!({"a": 2}), json!(0), json!(null)]
            ; "const_object")]
    #[test_case(json!({"const": false}),
            &[json!(false)],
            &[json!(true), json!(0), json!(null)]
            ; "const_boolean_false")]
    #[test_case(json!({"type": "object", "additionalProperties": {"type": "integer"}}),
            &[json!({}), json!({"x": 1})],
            &[json!({"x": "s"}), json!({"x": 1, "y": null})]
            ; "object_additional_properties_uses_not")]
    #[test_case(json!({"type": "object", "propertyNames": {"minLength": 2}}),
            &[json!({}), json!({"ab": 1})],
            &[json!({"a": 1})]
            ; "object_property_names_uses_not")]
    #[test_case(json!({"type": "object", "dependentRequired": {"a": ["b"]}}),
            &[json!({}), json!({"b": 1}), json!({"a": 1, "b": 2})],
            &[json!({"a": 1})]
            ; "object_dependent_required_uses_not")]
    #[test_case(json!({"type": "array", "prefixItems": [{"type": "integer"}], "items": {"type": "string"}}),
            &[json!([]), json!([1]), json!([1, "a", "b"])],
            &[json!(["x"]), json!([1, 2])]
            ; "array_prefix_and_tail_uses_not")]
    #[test_case(json!({"type": "array", "contains": {"type": "integer"}}),
            &[json!([1]), json!([1, "x"])],
            &[json!([]), json!(["x"]), json!(0)]
            ; "array_contains_uses_not")]
    #[test_case(json!({"type": "array", "prefixItems": [{"type": "integer"}], "unevaluatedItems": false}),
            &[json!([]), json!([1])],
            &[json!([1, 2]), json!(["x"])]
            ; "array_unevaluated_items_uses_not")]
    fn negate_admits_exactly_the_complement(
        schema: Value,
        original_accepts: &[Value],
        original_rejects: &[Value],
    ) {
        let negated = negated_value(&schema);
        let raw_validator = crate::validator_for(&schema).expect("original schema must compile");
        let neg_validator = crate::validator_for(&negated).expect("negated schema must compile");
        for value in original_accepts {
            assert!(raw_validator.is_valid(value));
            assert!(!neg_validator.is_valid(value));
        }
        for value in original_rejects {
            assert!(!raw_validator.is_valid(value));
            assert!(neg_validator.is_valid(value));
        }
    }

    #[test_case(Schema::Boolean(BooleanBounds::Any),
            &[json!(true), json!(false)],
            &[json!(0), json!(null), json!("x")]
            ; "boolean_any")]
    #[test_case(Schema::Boolean(BooleanBounds::JustTrue),
            &[json!(true)],
            &[json!(false), json!(0), json!(null)]
            ; "boolean_just_true")]
    #[test_case(Schema::Boolean(BooleanBounds::JustFalse),
            &[json!(false)],
            &[json!(true), json!(0), json!(null)]
            ; "boolean_just_false")]
    #[test_case(Schema::Enum(Vec::new()),
            &[],
            &[json!(0), json!(null), json!("x"), json!([])]
            ; "empty_enum_negates_to_true")]
    #[test_case(typed_group_string_enum(),
            &[json!("a"), json!("b")],
            &[json!("c"), json!(0), json!(null)]
            ; "typed_group")]
    #[test_case(type_guard_string_minlen(),
            &[json!(0), json!(null), json!("abc")],
            &[json!("a")]
            ; "type_guard")]
    #[test_case(one_of_integer_string(),
            &[json!(0), json!("x")],
            &[json!(0.5), json!(null), json!([])]
            ; "one_of")]
    #[test_case(if_then_else_ir(),
            &[json!(0), json!(5), json!("x")],
            &[json!(0.5), json!(null), json!([]), json!(true)]
            ; "if_then_else")]
    #[test_case(if_then_only_ir(),
            &[json!(0), json!(5), json!("x")],
            &[json!(0.5), json!(null), json!([]), json!(true)]
            ; "if_then_only_defaults_then_to_true")]
    #[test_case(if_else_only_ir(),
            &[json!(0), json!("x"), json!(null), json!(0.5)],
            &[]
            ; "if_else_only_defaults_else_to_true")]
    fn ir_negate_is_exact_complement(schema: Schema, accepts: &[Value], rejects: &[Value]) {
        assert_ir_complement(schema, accepts, rejects);
    }

    // Sub-trees without an IR-direct complement negate to a `Not` wrapper.
    #[test_case(json!({"$ref": "https://example.com/s"}) ; "external_ref")]
    #[test_case(json!({"$dynamicRef": "#node"}) ; "dynamic_ref")]
    fn unrepresentable_negates_to_not_wrapper(schema: Value) {
        let negated = canonicalize(&schema).negate();
        assert!(matches!(negated.as_schema(), Schema::Not(_)));
    }

    // Untyped `pattern` + `minLength`: the lone-pattern dual bails (bare pattern only), so the length window
    // drives the complement.
    #[test_case(json!({"pattern": "^a", "minLength": 2}),
        &[json!("ab"), json!("aaa"), json!(0), json!(null)],
        &[json!("a"), json!("b")]
        ; "string_pattern_with_minlength")]
    // Untyped `required` + `propertyNames`: the required dual bails on the extra `propertyNames` facet.
    #[test_case(json!({"required": ["a"], "propertyNames": {"minLength": 1}}),
        &[json!({"a": 1}), json!(0), json!(null), json!("x")],
        &[json!({}), json!({"b": 1})]
        ; "object_required_with_property_names")]
    // Leading `true` prefix slot is skipped during per-position prefix negation; the second slot is the real constraint.
    #[test_case(json!({"type": "array", "prefixItems": [true, {"type": "integer"}]}),
        &[json!([]), json!([1]), json!([1, 2]), json!(["x", 5])],
        &[json!([1, "x"]), json!(0), json!(null)]
        ; "array_prefix_with_true_slot")]
    // `uniqueItems` with `contains` is the non-clean path: the unique<->repeated dual can't apply, so it falls back.
    #[test_case(json!({"type": "array", "uniqueItems": true, "contains": {"type": "integer"}}),
        &[json!([1]), json!([1, 2, 3]), json!(["x", 1])],
        &[json!([1, 1]), json!([]), json!(0), json!(null)]
        ; "array_unique_with_contains_falls_back")]
    // `not uniqueItems` folds into a `repeated_items` leaf; the sibling `contains` makes it non-clean, so the
    // repeated ↔ unique dual bails to a fallback instead of swapping the flag.
    #[test_case(json!({"allOf": [
            {"type": "array", "contains": {"type": "integer"}},
            {"not": {"type": "array", "uniqueItems": true}}
        ]}),
        &[json!([1, 1]), json!([5, 5, "x"])],
        &[json!([1, 2]), json!([1]), json!(["a", "a"]), json!([]), json!(0), json!(null)]
        ; "array_repeated_with_contains_falls_back")]
    // Const at the i64 extremes: the singleton split needs `v - 1`/`v + 1` (unrepresentable here), so it falls
    // back to the `not(const)` complement rather than the bound-pair expansion.
    #[test_case(json!({"const": i64::MAX}),
        &[json!(i64::MAX)],
        &[json!(i64::MAX - 1), json!(0), json!("x"), json!(null)]
        ; "const_i64_max_falls_back")]
    #[test_case(json!({"const": i64::MIN}),
        &[json!(i64::MIN)],
        &[json!(i64::MIN + 1), json!(0), json!(null)]
        ; "const_i64_min_falls_back")]
    // A `u64` const above the i64 range: `as_i64()` is `None`, so the singleton split is skipped for the
    // `not(const)` fallback.
    #[test_case(json!({"const": 9_223_372_036_854_775_808_u64}),
        &[json!(9_223_372_036_854_775_808_u64)],
        &[json!(i64::MAX), json!(0), json!(null)]
        ; "const_u64_above_i64_falls_back")]
    // A `u64::MAX` length cap is vacuous, so the leaf normalizes to the open type and negates to "not array".
    #[test_case(json!({"type": "array", "maxItems": 18_446_744_073_709_551_615_u64}),
        &[json!([]), json!([1]), json!([1, 2, 3])],
        &[json!(0), json!("x"), json!(null), json!({}), json!(true)]
        ; "array_vacuous_max_items")]
    #[test_case(json!({"type": "object", "maxProperties": 18_446_744_073_709_551_615_u64}),
        &[json!({}), json!({"a": 1})],
        &[json!(0), json!([]), json!("x"), json!(null), json!(true)]
        ; "object_vacuous_max_properties")]
    // An all-`True` prefix is a non-open array leaf whose only facet negates to nothing in-kind, so
    // `wrap_in_kind` gets an empty branch list (complement is exactly the non-array types).
    #[test_case(json!({"type": "array", "prefixItems": [true, true]}),
        &[json!([]), json!([1]), json!([1, "x", 3])],
        &[json!(0), json!("x"), json!(null), json!({}), json!(true)]
        ; "array_all_true_prefix_empty_in_kind")]
    // Bounds at the i64 edges: JSON integers extend past the bound model, so the constructive
    // half-line duals are unsound there and the complement must keep an exact residual.
    #[test_case(json!({"type": "integer", "exclusiveMinimum": i64::MAX}),
        &[json!(9_223_372_036_854_775_808_u64)],
        &[json!(i64::MAX), json!(0), json!("x"), json!(null)]
        ; "integer_exclusive_minimum_at_i64_max")]
    #[test_case(json!({"type": "integer", "maximum": i64::MAX}),
        &[json!(0), json!(i64::MAX)],
        &[json!(10_000_000_000_000_000_000_u64), json!(1.5), json!("x")]
        ; "integer_maximum_at_i64_max")]
    #[test_case(json!({"type": "integer", "minimum": i64::MIN}),
        &[json!(0), json!(i64::MIN)],
        &[json!(-10_000_000_000_000_000_000.0), json!(1.5), json!("x")]
        ; "integer_minimum_at_i64_min")]
    #[test_case(json!({"type": "integer", "exclusiveMaximum": i64::MIN}),
        &[json!(-10_000_000_000_000_000_000.0)],
        &[json!(i64::MIN), json!(0), json!("x"), json!(null)]
        ; "integer_exclusive_maximum_at_i64_min")]
    fn negate_json_is_exact_complement(schema: Value, accepts: &[Value], rejects: &[Value]) {
        assert_json_complement(&schema, accepts, rejects);
    }
}
