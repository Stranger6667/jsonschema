#![cfg(not(target_arch = "wasm32"))]

use std::sync::LazyLock;

use hegel::{generators as gs, TestCase};
use serde_json::{json, Value};

use crate::{
    canonical::{context::CanonicalizationContext, coverage::covers},
    canonicalize,
};

// To hammer for new confluence classes, raise one property's `test_cases` (e.g. 200000) then restore
// to 1000; clear `crates/jsonschema/.hegel/examples` between runs so no seed replays at the lower budget.

// Small integer window so finite-domain reasoning stays decidable, and `enum`/`const` values overlap
// generated intervals so interval/value-set interactions are exercised.
fn bounded_integer(tc: &TestCase) -> i32 {
    tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))
}

// Spellings of one numeric value each. Under `arbitrary-precision` serde keeps the raw token, so
// value-set logic must equate them through normalization; without it they collapse to one f64 spelling.
const ALIAS_FAMILIES: &[&[&str]] = &[
    &["1.5", "1.50", "15e-1", "0.15e1", "1500e-3"],
    &["0.25", "2.5e-1", "25e-2", "0.025e1"],
    &["-0.5", "-5e-1", "-0.50", "-50e-2"],
    &["2", "2.0", "0.2e1", "20e-1", "2e0"],
];

fn alias_family(tc: &TestCase) -> &'static [&'static str] {
    tc.draw(gs::sampled_from(ALIAS_FAMILIES.to_vec()))
}

fn parse_spelling(spelling: &str) -> Value {
    serde_json::from_str(spelling).expect("valid number literal")
}

fn spelling_from(tc: &TestCase, family: &[&str]) -> Value {
    let index = tc.draw(
        gs::integers::<usize>()
            .min_value(0)
            .max_value(family.len() - 1),
    );
    parse_spelling(family[index])
}

fn aliased_number(tc: &TestCase) -> Value {
    let family = alias_family(tc);
    spelling_from(tc, family)
}

// A bounded scalar JSON value for `const`/`enum`, drawn across all primitive types.
#[hegel::composite]
fn arbitrary_scalar(tc: TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(5)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => Value::String(tc.draw(gs::text().max_size(3))),
        3 => aliased_number(&tc),
        _ => json!(bounded_integer(&tc)),
    }
}

// Pool of patterns incl. an unmatchable one (`a^` matches nothing) so regex-emptiness paths fire.
fn arbitrary_pattern(tc: &TestCase) -> &'static str {
    tc.draw(gs::sampled_from(vec!["^a", "b$", "^[a-z]+$", "a^", "^x"]))
}

#[hegel::composite]
fn arbitrary_leaf_schema(tc: TestCase) -> Value {
    let choice = tc.draw(gs::integers::<u8>().min_value(0).max_value(39));
    match choice {
        0 => json!({"type": "null"}),
        1 => json!({"type": "boolean"}),
        2 => json!({"type": "integer"}),
        3 => json!({"type": "string"}),
        4 => {
            let (minimum, maximum) = ordered(bounded_integer(&tc), bounded_integer(&tc));
            json!({"type": "integer", "minimum": minimum, "maximum": maximum})
        }
        5 => json!({"type": "object", "required": ["a"]}),
        6 => json!({"type": "object", "properties": {"a": {"type": "integer"}}}),
        7 => {
            let minimum = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
            let maximum = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
            let (minimum, maximum) = ordered(minimum, maximum);
            json!({"type": "object", "minProperties": minimum, "maxProperties": maximum})
        }
        8 => json!({"type": "object", "patternProperties": {"^x": {"type": "integer"}}}),
        9 => {
            let minimum = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
            let maximum = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
            let (minimum, maximum) = ordered(minimum, maximum);
            json!({"type": "array", "minItems": minimum, "maxItems": maximum})
        }
        10 => json!({"type": "array", "items": {"type": "integer"}}),
        11 => json!({"type": "array", "prefixItems": [{"type": "integer"}]}),
        12 => {
            let modulus = tc.draw(gs::integers::<u8>().min_value(1).max_value(7));
            json!({"type": "integer", "multipleOf": modulus})
        }
        13 => json!({"type": "array", "uniqueItems": true, "items": {"type": "boolean"}}),
        14 => json!({"type": "string", "pattern": "^a"}),
        15 => json!({"type": "object", "propertyNames": {"pattern": "^[a-z]+$"}}),
        16 => json!({"type": "object", "additionalProperties": {"type": "integer"}}),
        17 => {
            let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(4));
            let values: Vec<Value> = (0..count).map(|_| tc.draw(arbitrary_scalar())).collect();
            json!({"enum": values})
        }
        18 => {
            let modulus = tc.draw(gs::integers::<u8>().min_value(1).max_value(7));
            json!({"type": "number", "multipleOf": modulus})
        }
        19 => json!({"type": "integer", "exclusiveMinimum": bounded_integer(&tc)}),
        20 => {
            let minimum = tc.draw(gs::integers::<u8>().min_value(0).max_value(4));
            let maximum = tc.draw(gs::integers::<u8>().min_value(0).max_value(4));
            let (minimum, maximum) = ordered(minimum, maximum);
            json!({"type": "string", "minLength": minimum, "maxLength": maximum})
        }
        21 => {
            json!({"type": "object", "properties": {"a": {"type": "integer"}}, "additionalProperties": false})
        }
        22 => {
            let (minimum, maximum) = ordered(bounded_integer(&tc), bounded_integer(&tc));
            json!({"type": "number", "minimum": minimum, "maximum": maximum})
        }
        23 => {
            json!({"type": "string", "pattern": arbitrary_pattern(&tc)})
        }
        24 => {
            let format = match tc.draw(gs::integers::<u8>().min_value(0).max_value(3)) {
                0 => "email",
                1 => "uuid",
                2 => "date",
                _ => "date-time",
            };
            json!({"type": "string", "format": format})
        }
        25 => json!({"type": "object", "required": ["a"], "dependentRequired": {"a": ["b"]}}),
        26 => json!({"type": "object", "dependentSchemas": {"a": {"required": ["b"]}}}),
        27 => {
            let lower = tc.draw(gs::integers::<u8>().min_value(0).max_value(2));
            let upper = tc.draw(gs::integers::<u8>().min_value(0).max_value(2));
            let (lower, upper) = ordered(lower, upper);
            let max_items = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
            json!({
                "type": "array",
                "contains": {"type": "integer"},
                "minContains": lower,
                "maxContains": upper,
                "maxItems": max_items
            })
        }
        28 => {
            let types = match tc.draw(gs::integers::<u8>().min_value(0).max_value(3)) {
                0 => json!(["null", "boolean"]),
                1 => json!(["integer", "string"]),
                2 => json!(["number", "string"]),
                _ => json!(["null", "boolean", "string"]),
            };
            json!({"type": types})
        }
        29 => json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}},
            "patternProperties": {"^x": {"type": "string"}},
            "additionalProperties": {"type": "boolean"}
        }),
        30 => {
            json!({"type": "object", "propertyNames": {"pattern": arbitrary_pattern(&tc)}, "required": ["a"]})
        }
        // Type-less guards: facets without `type` constrain only their own kind and pass the rest.
        31 => json!({"pattern": "^a"}),
        32 => json!({"uniqueItems": true}),
        33 => {
            let modulus = tc.draw(gs::integers::<u8>().min_value(1).max_value(7));
            json!({"multipleOf": modulus})
        }
        34 => json!({"required": ["a"]}),
        35 => json!({"const": tc.draw(arbitrary_scalar())}),
        36 => {
            // Degenerate interval spelled two ways: pins the leaf to one value across spellings.
            let family = alias_family(&tc);
            json!({
                "type": "number",
                "minimum": spelling_from(&tc, family),
                "maximum": spelling_from(&tc, family)
            })
        }
        37 => {
            // `multipleOf` must be positive; skip the negative family.
            let index = tc.draw(gs::integers::<usize>().min_value(0).max_value(2));
            let family = ALIAS_FAMILIES[[0, 1, 3][index]];
            json!({"multipleOf": spelling_from(&tc, family)})
        }
        38 => json!({"const": aliased_number(&tc)}),
        _ => {
            // Whole enum from one alias family: spellings of one value collide in dedup, intersection,
            // and negation value-set paths.
            let family = alias_family(&tc);
            let count = tc.draw(gs::integers::<usize>().min_value(2).max_value(3));
            let values: Vec<Value> = (0..count).map(|_| spelling_from(&tc, family)).collect();
            json!({"enum": values})
        }
    }
}

fn ordered<T: PartialOrd>(left: T, right: T) -> (T, T) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

// Hegel rejects self-recursive composite generators; assemble bottom-up. Ref-free: symbolic
// `Reference`/`Recursive` leaves are opaque to the algebra, so the algebraic-law properties assert
// over this fragment only.
#[hegel::composite]
fn arbitrary_structural_schema(tc: TestCase) -> Value {
    let depth = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
    let mut nodes: Vec<Value> = tc.draw(gs::vecs(arbitrary_leaf_schema()).min_size(1).max_size(8));
    for _ in 0..depth {
        let mut next: Vec<Value> = Vec::new();
        let group_count = tc.draw(gs::integers::<usize>().min_value(1).max_value(nodes.len()));
        let mut cursor = 0;
        for _ in 0..group_count {
            let remaining = nodes.len() - cursor;
            if remaining == 0 {
                break;
            }
            let take = tc.draw(
                gs::integers::<usize>()
                    .min_value(1)
                    .max_value(remaining.min(3)),
            );
            let branches: Vec<Value> = nodes[cursor..cursor + take].to_vec();
            cursor += take;
            let kind = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
            let combined = match kind {
                0 => json!({"allOf": branches}),
                1 => json!({"anyOf": branches}),
                2 => json!({"not": branches[0].clone()}),
                _ => {
                    let mut branches = branches;
                    // An inert `false` branch never matches, so `oneOf` must canonicalize as if it were absent.
                    if tc.draw(gs::booleans()) {
                        branches.push(json!(false));
                    }
                    json!({"oneOf": branches})
                }
            };
            next.push(combined);
        }
        if cursor < nodes.len() {
            next.extend(nodes[cursor..].iter().cloned());
        }
        nodes = next;
    }
    nodes.into_iter().next().unwrap_or(json!(true))
}

#[hegel::composite]
fn arbitrary_schema_root(tc: TestCase) -> Value {
    let root = tc.draw(arbitrary_structural_schema());
    // Occasionally wrap with `$defs`/`$ref`: an acyclic shared definition or a recursive binding.
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(9)) {
        0 => json!({
            "type": "object",
            "$defs": {"shared": root},
            "properties": {"a": {"$ref": "#/$defs/shared"}, "b": {"$ref": "#/$defs/shared"}}
        }),
        1 => json!({
            "$defs": {"node": {
                "type": "object",
                "properties": {"next": {"$ref": "#/$defs/node"}, "value": root}
            }},
            "$ref": "#/$defs/node"
        }),
        _ => root,
    }
}

#[hegel::composite]
fn arbitrary_json_value(tc: TestCase) -> Value {
    let choice = tc.draw(gs::integers::<u8>().min_value(0).max_value(6));
    match choice {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        // Within the f64-exact integer range: the runtime validator's numeric checks are f64-based,
        // so past ±2^53 its verdicts diverge from the exact algebra (e.g. `multipleOf: 1.5` on an
        // odd multiple of 3) and validation-parity properties cannot hold.
        2 => json!(tc.draw(
            gs::integers::<i64>()
                .min_value(-(1 << 53))
                .max_value(1 << 53)
        )),
        3 => Value::String(tc.draw(gs::text().max_size(16))),
        6 => aliased_number(&tc),
        4 => {
            let pick = tc.draw(gs::integers::<u8>().min_value(0).max_value(5));
            match pick {
                0 => json!({}),
                1 => json!({"a": 1}),
                2 => json!({"a": "not-int"}),
                3 => json!({"b": 1, "c": 2}),
                4 => json!({"x1": 7}),
                _ => json!({"x1": "bad", "y": 0}),
            }
        }
        _ => {
            let pick = tc.draw(gs::integers::<u8>().min_value(0).max_value(2));
            match pick {
                0 => json!([]),
                1 => json!([1, 2]),
                _ => json!([null, "x"]),
            }
        }
    }
}

#[hegel::test(test_cases = 1000)]
fn canonicalize_is_idempotent(tc: TestCase) {
    let schema = tc.draw(arbitrary_schema_root());
    let once = canonicalize(&schema).expect("valid generated schema");
    let twice = canonicalize(&once.to_json_schema()).expect("re-canonicalize failed");
    assert_eq!(
        once,
        twice,
        "canonicalize must be idempotent\n  input={schema}\n  first={}\n  second={}",
        once.to_json_schema(),
        twice.to_json_schema(),
    );
}

// Value-set interactions where each side spells one number differently; the general schema generator
// reaches these combinations too rarely to fence the bug class on its own.
#[hegel::composite]
fn aliased_value_set_schema(tc: TestCase) -> Value {
    let family = alias_family(&tc);
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(4)) {
        0 => json!({"allOf": [
            {"const": spelling_from(&tc, family)},
            {"enum": [spelling_from(&tc, family), "x"]}
        ]}),
        1 => json!({"oneOf": [
            {"const": spelling_from(&tc, family)},
            {"const": spelling_from(&tc, family)}
        ]}),
        2 => json!({"allOf": [
            {"enum": [spelling_from(&tc, family), true]},
            {"enum": [spelling_from(&tc, family), true]}
        ]}),
        3 => json!({"allOf": [
            {"type": "number", "minimum": spelling_from(&tc, family), "maximum": spelling_from(&tc, family)},
            {"enum": [spelling_from(&tc, family)]}
        ]}),
        _ => json!({"allOf": [
            {"type": "number"},
            {"not": {"const": spelling_from(&tc, family)}},
            {"enum": [spelling_from(&tc, family), 7]}
        ]}),
    }
}

// Every spelling of every family: parity witnesses that hit the aliased values regardless of which
// spellings the schema drew.
static ALIAS_WITNESSES: LazyLock<Vec<Value>> = LazyLock::new(|| {
    ALIAS_FAMILIES
        .iter()
        .flat_map(|family| family.iter())
        .map(|spelling| parse_spelling(spelling))
        .chain([json!("x"), json!(true), json!(7), Value::Null])
        .collect()
});

fn assert_validation_parity(schema: &Value, instances: &[Value]) {
    let canonical = canonicalize(schema).expect("valid generated schema");
    let canonical_value = canonical.to_json_schema();
    let validator_raw = crate::validator_for(schema).expect("raw compiles");
    let validator_canonical = crate::validator_for(&canonical_value).expect("canonical compiles");
    for instance in instances {
        assert_eq!(
            validator_raw.is_valid(instance),
            validator_canonical.is_valid(instance),
            "validation must agree: schema={schema}, canonical={canonical_value}, instance={instance}",
        );
    }
}

#[hegel::test(test_cases = 1000)]
fn aliased_value_sets_preserve_validation_semantics(tc: TestCase) {
    let schema = tc.draw(aliased_value_set_schema());
    assert_validation_parity(&schema, &ALIAS_WITNESSES);
}

#[hegel::test(test_cases = 1000)]
fn canonicalize_preserves_validation_semantics(tc: TestCase) {
    let schema = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    assert_validation_parity(&schema, &instances);
}

#[hegel::test(test_cases = 1000)]
fn intersect_is_sound(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    let intersection = canonical_left.intersect(&canonical_right).to_json_schema();
    let validator_left = crate::validator_for(&left).expect("left compiles");
    let validator_right = crate::validator_for(&right).expect("right compiles");
    let validator_inter = crate::validator_for(&intersection).expect("intersection compiles");
    for instance in &instances {
        let in_left = validator_left.is_valid(instance);
        let in_right = validator_right.is_valid(instance);
        let in_intersection = validator_inter.is_valid(instance);
        if in_intersection {
            assert!(
                in_left && in_right,
                "intersect accepts an instance one input rejects\n  left={left}\n  right={right}\n  intersection={intersection}\n  instance={instance}",
            );
        }
    }
}

#[hegel::test(test_cases = 1000)]
fn intersect_is_commutative_modulo_canonicalize(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    let left_right = canonicalize(&canonical_left.intersect(&canonical_right).to_json_schema())
        .expect("left-right canonicalize");
    let right_left = canonicalize(&canonical_right.intersect(&canonical_left).to_json_schema())
        .expect("right-left canonicalize");
    assert_eq!(
        left_right, right_left,
        "intersect must be commutative up to canonical form; left={left}, right={right}",
    );
}

// Union is exact: it accepts an instance iff either input accepts it.
#[hegel::test(test_cases = 1000)]
fn union_is_exact(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    let union = canonical_left.union(&canonical_right).to_json_schema();
    let validator_left = crate::validator_for(&left).expect("left compiles");
    let validator_right = crate::validator_for(&right).expect("right compiles");
    let validator_union = crate::validator_for(&union).expect("union compiles");
    for instance in &instances {
        let in_either = validator_left.is_valid(instance) || validator_right.is_valid(instance);
        assert_eq!(
            validator_union.is_valid(instance),
            in_either,
            "union disagrees with its operands\n  left={left}\n  right={right}\n  union={union}\n  instance={instance}",
        );
    }
}

#[hegel::test(test_cases = 1000)]
fn union_is_commutative_modulo_canonicalize(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    let left_right = canonicalize(&canonical_left.union(&canonical_right).to_json_schema())
        .expect("left-right canonicalize");
    let right_left = canonicalize(&canonical_right.union(&canonical_left).to_json_schema())
        .expect("right-left canonicalize");
    assert_eq!(
        left_right, right_left,
        "union must be commutative up to canonical form; left={left}, right={right}",
    );
}

#[ignore = "known completeness gap: union-coverage between composed branches is grouping-sensitive; un-ignore when the coverage oracle closes the class"]
#[hegel::test(test_cases = 1000)]
fn intersect_is_associative_modulo_canonicalize(tc: TestCase) {
    let first = tc.draw(arbitrary_structural_schema());
    let second = tc.draw(arbitrary_structural_schema());
    let third = tc.draw(arbitrary_structural_schema());
    let a = canonicalize(&first).expect("valid first");
    let b = canonicalize(&second).expect("valid second");
    let c = canonicalize(&third).expect("valid third");
    let grouped_left = canonicalize(&a.intersect(&b).intersect(&c).to_json_schema())
        .expect("grouped-left canonicalize");
    let grouped_right = canonicalize(&a.intersect(&b.intersect(&c)).to_json_schema())
        .expect("grouped-right canonicalize");
    assert_eq!(
        grouped_left, grouped_right,
        "intersect must be associative up to canonical form; first={first}, second={second}, third={third}",
    );
}

#[ignore = "known completeness gap: union-coverage between composed branches is grouping-sensitive; un-ignore when the coverage oracle closes the class"]
#[hegel::test(test_cases = 1000)]
fn union_is_associative_modulo_canonicalize(tc: TestCase) {
    let first = tc.draw(arbitrary_structural_schema());
    let second = tc.draw(arbitrary_structural_schema());
    let third = tc.draw(arbitrary_structural_schema());
    let a = canonicalize(&first).expect("valid first");
    let b = canonicalize(&second).expect("valid second");
    let c = canonicalize(&third).expect("valid third");
    let grouped_left =
        canonicalize(&a.union(&b).union(&c).to_json_schema()).expect("grouped-left canonicalize");
    let grouped_right =
        canonicalize(&a.union(&b.union(&c)).to_json_schema()).expect("grouped-right canonicalize");
    assert_eq!(
        grouped_left, grouped_right,
        "union must be associative up to canonical form; first={first}, second={second}, third={third}",
    );
}

// Absorption: `a ∧ (a ∨ b)` collapses back to `a`.
#[ignore = "known completeness gap: union-coverage between composed branches is grouping-sensitive; un-ignore when the coverage oracle closes the class"]
#[hegel::test(test_cases = 1000)]
fn intersect_absorbs_own_union_modulo_canonicalize(tc: TestCase) {
    let first = tc.draw(arbitrary_structural_schema());
    let second = tc.draw(arbitrary_structural_schema());
    let a = canonicalize(&first).expect("valid first");
    let b = canonicalize(&second).expect("valid second");
    let absorbed =
        canonicalize(&a.intersect(&a.union(&b)).to_json_schema()).expect("absorbed canonicalize");
    let baseline = canonicalize(&a.to_json_schema()).expect("baseline canonicalize");
    assert_eq!(
        absorbed, baseline,
        "a ∧ (a ∨ b) must collapse to a; first={first}, second={second}",
    );
}

#[hegel::test(test_cases = 1000)]
fn intersect_with_self_is_identity_modulo_canonicalize(tc: TestCase) {
    let schema = tc.draw(arbitrary_structural_schema());
    let a = canonicalize(&schema).expect("valid generated schema");
    let reduced = canonicalize(&a.intersect(&a).to_json_schema()).expect("reduced canonicalize");
    let baseline = canonicalize(&a.to_json_schema()).expect("baseline canonicalize");
    assert_eq!(
        reduced, baseline,
        "a ∧ a must collapse to a; schema={schema}"
    );
}

#[hegel::test(test_cases = 1000)]
fn union_with_self_is_identity_modulo_canonicalize(tc: TestCase) {
    let schema = tc.draw(arbitrary_structural_schema());
    let a = canonicalize(&schema).expect("valid generated schema");
    let reduced = canonicalize(&a.union(&a).to_json_schema()).expect("reduced canonicalize");
    let baseline = canonicalize(&a.to_json_schema()).expect("baseline canonicalize");
    assert_eq!(
        reduced, baseline,
        "a ∨ a must collapse to a; schema={schema}"
    );
}

#[hegel::test(test_cases = 1000)]
fn negate_complements_validation(tc: TestCase) {
    let schema = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical = canonicalize(&schema).expect("valid generated schema");
    let negated = canonical.negate().to_json_schema();
    let raw_validator = crate::validator_for(&schema).expect("raw compiles");
    let neg_validator = crate::validator_for(&negated).expect("negated compiles");
    for instance in &instances {
        assert_eq!(
            raw_validator.is_valid(instance),
            !neg_validator.is_valid(instance),
            "negate must produce the exact complement\n  schema   = {schema}\n  negated  = {negated}\n  instance = {instance}",
        );
    }
}

// The involution target: green here means negate's output lands inside the canonical fragment for
// every generated shape. Blocked today by negate emitting symbolic `Not` residuals, same-kind
// unions, and `AnyOf`-of-`AllOf` reconstructions of `oneOf`/`if`-`then`-`else`.
#[ignore = "known completeness gaps: negate output escapes the canonical fragment; un-ignore per-class as they close"]
#[hegel::test(test_cases = 1000)]
fn double_negation_is_structural_identity(tc: TestCase) {
    let schema = tc.draw(arbitrary_structural_schema());
    let canonical = canonicalize(&schema).expect("valid generated schema");
    let round_tripped = canonical.negate().negate();
    assert_eq!(
        canonical, round_tripped,
        "negate must be an involution up to canonical form; schema={schema}",
    );
}

#[hegel::test(test_cases = 1000)]
fn double_negation_preserves_semantics(tc: TestCase) {
    let schema = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical = canonicalize(&schema).expect("valid generated schema");
    let double_negated = canonical.negate().negate().to_json_schema();
    let raw_validator = crate::validator_for(&schema).expect("raw compiles");
    let double_negated_validator =
        crate::validator_for(&double_negated).expect("double-negated compiles");
    for instance in &instances {
        assert_eq!(
            raw_validator.is_valid(instance),
            double_negated_validator.is_valid(instance),
            "negate(negate(x)) must accept the same instances\n  schema         = {schema}\n  double_negated = {double_negated}\n  instance       = {instance}",
        );
    }
}

// `is_satisfiable()` is sound: if it reports empty, the original schema must reject every value.
// Validates against the source (not the emitted canonical) so a wrong collapse to `false` can't self-confirm.
#[hegel::test(test_cases = 1000)]
fn unsatisfiable_is_sound(tc: TestCase) {
    let schema = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical = canonicalize(&schema).expect("valid generated schema");
    if canonical.is_satisfiable() {
        return;
    }
    let validator = crate::validator_for(&schema).expect("schema compiles");
    for instance in &instances {
        assert!(
            !validator.is_valid(instance),
            "is_satisfiable()=false but the schema accepts an instance\n  schema   = {schema}\n  instance = {instance}",
        );
    }
}

// `subtract` is the exact set difference: a value matches `A \ B` iff it matches `A` and not `B`.
// Validates the residual against the original inputs so a faulty negate/intersect cannot self-confirm.
#[hegel::test(test_cases = 1000)]
fn subtract_is_exact_set_difference(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    let residual = canonical_left.subtract(&canonical_right).to_json_schema();
    let validator_left = crate::validator_for(&left).expect("left compiles");
    let validator_right = crate::validator_for(&right).expect("right compiles");
    let validator_residual = crate::validator_for(&residual).expect("residual compiles");
    for instance in &instances {
        let expected = validator_left.is_valid(instance) && !validator_right.is_valid(instance);
        assert_eq!(
            validator_residual.is_valid(instance),
            expected,
            "subtract is not exact set difference\n  left={left}\n  right={right}\n  residual={residual}\n  instance={instance}",
        );
    }
}

// `is_subschema_of` is sound in the `true` direction: `Some(true)` means no value matches `A` but not `B`.
// A counterexample search with the runtime validators catches any unsound `Some(true)` (incl. a faulty `covers`).
#[hegel::test(test_cases = 1000)]
fn is_subschema_of_true_is_sound(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    if canonical_left.is_subschema_of(&canonical_right) != Some(true) {
        return;
    }
    let validator_left = crate::validator_for(&left).expect("left compiles");
    let validator_right = crate::validator_for(&right).expect("right compiles");
    for instance in &instances {
        assert!(
            !validator_left.is_valid(instance) || validator_right.is_valid(instance),
            "is_subschema_of returned Some(true) but a value matches left and not right\n  left={left}\n  right={right}\n  instance={instance}",
        );
    }
}

// `covers` is the conservative containment oracle behind the `Some(true)` fast path: a `true` verdict
// must never admit a value of `small` that `big` rejects.
#[hegel::test(test_cases = 1000)]
fn covers_true_admits_no_separating_witness(tc: TestCase) {
    let big = tc.draw(arbitrary_schema_root());
    let small = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical_big = canonicalize(&big).expect("valid big");
    let canonical_small = canonicalize(&small).expect("valid small");
    let ctx = CanonicalizationContext::default();
    if !covers(canonical_big.as_shared(), canonical_small.as_shared(), &ctx) {
        return;
    }
    let big_validator = crate::validator_for(&big).expect("big compiles");
    let small_validator = crate::validator_for(&small).expect("small compiles");
    for instance in &instances {
        assert!(
            !small_validator.is_valid(instance) || big_validator.is_valid(instance),
            "covers returned true but a value separates them\n  big={big}\n  small={small}\n  instance={instance}",
        );
    }
}

// A `Some(false)` verdict stands on a decidably-inhabited residual, so it must be satisfiable. (The
// `Some(true)` direction is not a theorem; its soundness is in `is_subschema_of_true_is_sound`.)
#[hegel::test(test_cases = 1000)]
fn is_subschema_of_false_implies_satisfiable_residual(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    if canonical_left.is_subschema_of(&canonical_right) == Some(false) {
        assert!(
            canonical_left.subtract(&canonical_right).is_satisfiable(),
            "Some(false) but residual is empty\n  left={left}\n  right={right}",
        );
    }
}

// A refutation and a containment proof are mutually exclusive: `Some(false)` asserts a separating
// witness exists, a `covers` proof asserts none can exist.
#[hegel::test(test_cases = 1000)]
fn is_subschema_of_false_implies_not_covered(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    if canonical_left.is_subschema_of(&canonical_right) == Some(false) {
        let ctx = CanonicalizationContext::default();
        assert!(
            !covers(
                canonical_right.as_shared(),
                canonical_left.as_shared(),
                &ctx
            ),
            "Some(false) verdict contradicts a covers proof\n  left={left}\n  right={right}",
        );
    }
}

// A finite integer set spelled as a flat `enum` and as a `multipleOf` window plus outliers must
// canonicalize identically: both collapse to one leaf when the set is exactly one shape, else one value set.
#[hegel::test(test_cases = 1000)]
fn finite_integer_set_spellings_converge(tc: TestCase) {
    let step = tc.draw(gs::integers::<i64>().min_value(2).max_value(4));
    let count = tc.draw(gs::integers::<i64>().min_value(2).max_value(8));
    let start = tc.draw(gs::integers::<i64>().min_value(-4).max_value(4)) * step;
    let window: Vec<i64> = (0..count).map(|i| start + i * step).collect();
    let mut outliers: Vec<i64> = tc
        .draw(gs::vecs(gs::integers::<i64>().min_value(-12).max_value(12)).max_size(4))
        .into_iter()
        .filter(|value| !window.contains(value))
        .collect();
    outliers.sort_unstable();
    outliers.dedup();

    let mut members: Vec<i64> = window.iter().chain(outliers.iter()).copied().collect();
    members.sort_unstable();
    members.dedup();

    let high = start + (count - 1) * step;
    let window_leaf =
        json!({"type": "integer", "minimum": start, "maximum": high, "multipleOf": step});
    let flat = json!({ "enum": members });
    let split = if outliers.is_empty() {
        window_leaf
    } else {
        json!({ "anyOf": [window_leaf, { "enum": outliers }] })
    };

    let canonical_flat = canonicalize(&flat).expect("flat canonicalize");
    let canonical_split = canonicalize(&split).expect("split canonicalize");
    assert_eq!(
        canonical_flat, canonical_split,
        "finite integer set spellings diverge; flat={flat}, split={split}",
    );
}

// Same-kind leaf schema, exercising one `Leaf::covers` implementation directly.
fn kind_leaf(tc: &TestCase, kind: u8) -> Value {
    let pick = tc.draw(gs::integers::<u8>().min_value(0).max_value(3));
    match (kind, pick) {
        (0, 0) => json!({"type": "integer"}),
        (0, 1) => {
            let (minimum, maximum) = ordered(bounded_integer(tc), bounded_integer(tc));
            json!({"type": "integer", "minimum": minimum, "maximum": maximum})
        }
        (0, 2) => {
            let modulus = tc.draw(gs::integers::<u8>().min_value(1).max_value(7));
            json!({"type": "integer", "multipleOf": modulus})
        }
        (0, _) => json!({"type": "integer", "exclusiveMinimum": bounded_integer(tc)}),
        (1, 0) => json!({"type": "number"}),
        (1, 1) => {
            let (minimum, maximum) = ordered(bounded_integer(tc), bounded_integer(tc));
            json!({"type": "number", "minimum": minimum, "maximum": maximum})
        }
        (1, _) => {
            let modulus = tc.draw(gs::integers::<u8>().min_value(1).max_value(7));
            json!({"type": "number", "multipleOf": modulus})
        }
        (2, 0) => json!({"type": "string"}),
        (2, 1) => json!({"type": "string", "pattern": arbitrary_pattern(tc)}),
        (2, 2) => {
            let (minimum, maximum) = ordered(
                tc.draw(gs::integers::<u8>().min_value(0).max_value(4)),
                tc.draw(gs::integers::<u8>().min_value(0).max_value(4)),
            );
            json!({"type": "string", "minLength": minimum, "maxLength": maximum})
        }
        (2, _) => json!({"type": "string", "format": "email"}),
        (3, 0) => {
            let (minimum, maximum) = ordered(
                tc.draw(gs::integers::<u8>().min_value(0).max_value(3)),
                tc.draw(gs::integers::<u8>().min_value(0).max_value(3)),
            );
            json!({"type": "array", "minItems": minimum, "maxItems": maximum})
        }
        (3, 1) => json!({"type": "array", "items": {"type": "integer"}}),
        (3, 2) => json!({"type": "array", "prefixItems": [{"type": "integer"}]}),
        (3, _) => {
            json!({"type": "array", "contains": {"type": "integer"}, "minContains": 1, "maxItems": 3})
        }
        (_, 0) => json!({"type": "object", "required": ["a"]}),
        (_, 1) => json!({"type": "object", "properties": {"a": {"type": "integer"}}}),
        (_, 2) => {
            json!({"type": "object", "properties": {"a": {"type": "integer"}}, "additionalProperties": false})
        }
        (_, _) => {
            json!({"type": "object", "patternProperties": {"^x": {"type": "integer"}}, "maxProperties": 2})
        }
    }
}

// Instances of the matching kind so sampled witnesses can actually separate same-kind leaves.
fn kind_instance(tc: &TestCase, kind: u8) -> Value {
    match kind {
        0 => json!(bounded_integer(tc)),
        1 => match tc.draw(gs::integers::<u8>().min_value(0).max_value(1)) {
            0 => json!(bounded_integer(tc)),
            _ => json!(f64::from(bounded_integer(tc)) + 0.5),
        },
        2 => {
            let pool = ["", "a", "ab", "abc", "x", "xyz", "b", "A1"];
            let index = tc.draw(
                gs::integers::<usize>()
                    .min_value(0)
                    .max_value(pool.len() - 1),
            );
            json!(pool[index])
        }
        3 => match tc.draw(gs::integers::<u8>().min_value(0).max_value(4)) {
            0 => json!([]),
            1 => json!([1]),
            2 => json!([1, 2]),
            3 => json!([true]),
            _ => json!(["x", 1, 2]),
        },
        _ => match tc.draw(gs::integers::<u8>().min_value(0).max_value(4)) {
            0 => json!({}),
            1 => json!({"a": 1}),
            2 => json!({"a": "not-int"}),
            3 => json!({"x1": 7, "y": 0}),
            _ => json!({"a": 1, "b": 2, "c": 3}),
        },
    }
}

// Per-domain covers soundness: a Proven leaf `covers` must admit no kind-typed witness in small \ big
// (the syntactic verdict implies semantic containment of the concretized sets).
#[hegel::test(test_cases = 1000)]
fn leaf_covers_proven_is_sound_per_domain(tc: TestCase) {
    let kind = tc.draw(gs::integers::<u8>().min_value(0).max_value(4));
    let big = kind_leaf(&tc, kind);
    let small = kind_leaf(&tc, kind);
    let instances: Vec<Value> = (0..8).map(|_| kind_instance(&tc, kind)).collect();
    let canonical_big = canonicalize(&big).expect("valid big");
    let canonical_small = canonicalize(&small).expect("valid small");
    let ctx = CanonicalizationContext::default();
    if !covers(canonical_big.as_shared(), canonical_small.as_shared(), &ctx) {
        return;
    }
    let big_validator = crate::validator_for(&big).expect("big compiles");
    let small_validator = crate::validator_for(&small).expect("small compiles");
    for instance in &instances {
        assert!(
            !small_validator.is_valid(instance) || big_validator.is_valid(instance),
            "leaf covers returned true but a value separates them\n  big={big}\n  small={small}\n  instance={instance}",
        );
    }
}

// s ∩ not s must reject every value.
#[hegel::test(test_cases = 1000)]
fn schema_and_its_negation_intersect_to_empty(tc: TestCase) {
    let schema = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical = canonicalize(&schema).expect("valid generated schema");
    let negated = canonical.negate();
    let intersection_value = canonical.intersect(&negated).to_json_schema();
    let validator = crate::validator_for(&intersection_value).expect("intersection compiles");
    for instance in &instances {
        assert!(
            !validator.is_valid(instance),
            "schema ∩ negate(schema) must be empty\n  schema       = {schema}\n  intersection = {intersection_value}\n  instance     = {instance}",
        );
    }
}

// A∩B must accept every instance both A and B accept (completeness; `intersect_is_sound` checks soundness).
#[hegel::test(test_cases = 1000)]
fn intersect_is_complete(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    let intersection = canonical_left.intersect(&canonical_right).to_json_schema();
    let validator_left = crate::validator_for(&left).expect("left compiles");
    let validator_right = crate::validator_for(&right).expect("right compiles");
    let validator_inter = crate::validator_for(&intersection).expect("intersection compiles");
    for instance in &instances {
        if validator_left.is_valid(instance) && validator_right.is_valid(instance) {
            assert!(
                validator_inter.is_valid(instance),
                "intersect rejected an instance both inputs accept\n  left={left}\n  right={right}\n  intersection={intersection}\n  instance={instance}",
            );
        }
    }
}

// De Morgan: ¬(A∪B) must agree with ¬A∩¬B.
#[hegel::test(test_cases = 1000)]
fn de_morgan_union(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    let union_negated = canonical_left
        .union(&canonical_right)
        .negate()
        .to_json_schema();
    let neg_a_intersect_neg_b = canonical_left
        .negate()
        .intersect(&canonical_right.negate())
        .to_json_schema();
    let v_lhs = crate::validator_for(&union_negated).expect("¬(A∪B) compiles");
    let v_rhs = crate::validator_for(&neg_a_intersect_neg_b).expect("¬A∩¬B compiles");
    for instance in &instances {
        assert_eq!(
            v_lhs.is_valid(instance),
            v_rhs.is_valid(instance),
            "De Morgan ¬(A∪B)==¬A∩¬B violated\n  left={left}\n  right={right}\n  instance={instance}",
        );
    }
}

// De Morgan: ¬(A∩B) must agree with ¬A∪¬B.
#[hegel::test(test_cases = 1000)]
fn de_morgan_intersection(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let instances = tc.draw(gs::vecs(arbitrary_json_value()).min_size(1).max_size(10));
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    let intersection_negated = canonical_left
        .intersect(&canonical_right)
        .negate()
        .to_json_schema();
    let neg_a_union_neg_b = canonical_left
        .negate()
        .union(&canonical_right.negate())
        .to_json_schema();
    let v_lhs = crate::validator_for(&intersection_negated).expect("¬(A∩B) compiles");
    let v_rhs = crate::validator_for(&neg_a_union_neg_b).expect("¬A∪¬B compiles");
    for instance in &instances {
        assert_eq!(
            v_lhs.is_valid(instance),
            v_rhs.is_valid(instance),
            "De Morgan ¬(A∩B)==¬A∪¬B violated\n  left={left}\n  right={right}\n  instance={instance}",
        );
    }
}

// If either A or B is satisfiable, A∪B must be satisfiable.
#[hegel::test(test_cases = 1000)]
fn union_preserves_satisfiability(tc: TestCase) {
    let left = tc.draw(arbitrary_schema_root());
    let right = tc.draw(arbitrary_schema_root());
    let canonical_left = canonicalize(&left).expect("valid left");
    let canonical_right = canonicalize(&right).expect("valid right");
    if canonical_left.is_satisfiable() || canonical_right.is_satisfiable() {
        assert!(
            canonical_left.union(&canonical_right).is_satisfiable(),
            "union of a satisfiable schema must be satisfiable\n  left={left}\n  right={right}",
        );
    }
}
