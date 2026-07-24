#![cfg(not(target_arch = "wasm32"))]
use hegel::{extras::serde_json as json_gs, generators as gs, TestCase};
use jsonschema::{canonical::CanonicalView, Draft};
use serde_json::{json, Value};

fn draw_draft(tc: &TestCase) -> Draft {
    tc.draw(gs::sampled_from(vec![
        Draft::Draft4,
        Draft::Draft6,
        Draft::Draft7,
        Draft::Draft201909,
        Draft::Draft202012,
    ]))
}

fn draw_type(tc: &TestCase) -> &'static str {
    tc.draw(gs::sampled_from(vec![
        "null", "boolean", "integer", "number", "string", "array", "object",
    ]))
}

fn small_length(tc: &TestCase) -> u8 {
    tc.draw(gs::integers::<u8>().min_value(0).max_value(4))
}

fn small_int(tc: &TestCase) -> i32 {
    tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))
}

// Divisors spanning each arithmetic `multipleOf` compiles to: exact modulo, rational division, and
// the spellings on either side of the precision where they part ways.
const DIVISORS: &[&str] = &[
    "1",
    "2",
    "3",
    "0.5",
    "0.75",
    "1.5",
    "0.25",
    "0.1",
    "0.123456789",
    // fractional divisors whose common multiple with a whole one is itself whole
    "2.5",
    "1.25",
    "7.5",
    "0.2",
    "4503599627370496",
    "9007199254740992",
    "9007199254740993",
    "3002399751580331",
];

fn divisor(tc: &TestCase) -> Value {
    let text = tc.draw(gs::sampled_from(DIVISORS.to_vec()));
    serde_json::from_str(text).expect("valid number literal")
}

// Integers on both sides of exact `f64` precision, where a rewritten divisor changes the verdict.
const WIDE_INTEGERS: &[&str] = &[
    "9007199254740992",
    "9007199254740993",
    "18014398509481986",
    "27021597764222976",
    "27021597764222977",
    "12345678900000001",
    "13510798882111488",
    "1e30",
];

fn wide_number(tc: &TestCase) -> Value {
    let text = tc.draw(gs::sampled_from(WIDE_INTEGERS.to_vec()));
    serde_json::from_str(text).expect("valid number literal")
}

fn ordered<T: Ord>(a: T, b: T) -> (T, T) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn finite_float(tc: &TestCase) -> f64 {
    tc.draw(gs::floats::<f64>().min_value(-8.0).max_value(8.0))
}

// One value per family, spelled several ways: normalization must equate them (and, under
// `arbitrary-precision` where serde keeps the raw token, integer-valued floats fold to integers).
const ALIAS_FAMILIES: &[&[&str]] = &[
    &["1.5", "1.50", "15e-1"],
    &["2", "2.0", "0.2e1", "2e0"],
    &["-0.5", "-5e-1", "-0.50"],
    &["0", "0.0", "-0", "0e0"],
];

fn aliased_number(tc: &TestCase) -> Value {
    let family = tc.draw(gs::sampled_from(ALIAS_FAMILIES.to_vec()));
    let index = tc.draw(
        gs::integers::<usize>()
            .min_value(0)
            .max_value(family.len() - 1),
    );
    serde_json::from_str(family[index]).expect("valid number literal")
}

// A bounded scalar for `const`/`enum`, across the primitive types.
#[hegel::composite]
fn arbitrary_scalar(tc: TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(5)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => Value::String(tc.draw(gs::text().max_size(3))),
        3 => json!(tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))),
        4 => json!(finite_float(&tc)),
        _ => aliased_number(&tc),
    }
}

#[hegel::composite]
fn arbitrary_instance(tc: TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(10)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => json!(tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))),
        3 => json!(finite_float(&tc)),
        // An integer-valued float (`2.0`): Draft 4 treats it as a non-integer, later drafts as an integer.
        4 => json!(f64::from(
            tc.draw(gs::integers::<i32>().min_value(-4).max_value(4))
        )),
        5 => Value::String(tc.draw(gs::text().max_size(5))),
        6 => wide_number(&tc),
        7 => json!([]),
        8 => {
            let mut object = serde_json::Map::new();
            for key in draw_keys(&tc) {
                object.insert(key.to_string(), tc.draw(arbitrary_scalar()));
            }
            Value::Object(object)
        }
        9 => {
            let count = tc.draw(gs::integers::<usize>().min_value(0).max_value(2));
            Value::Array((0..count).map(|_| tc.draw(arbitrary_scalar())).collect())
        }
        _ => json!({}),
    }
}

// A modeled leaf: value sets, type sets, string facets, integer interval bounds, and container sizes.
fn draw_leaf(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(60)) {
        0 => json!({}),
        1 => json!(true),
        2 => json!(false),
        3 => json!({ "type": draw_type(tc) }),
        4 => json!({ "const": tc.draw(arbitrary_scalar()) }),
        5 => {
            let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(3));
            let values: Vec<Value> = (0..count).map(|_| tc.draw(arbitrary_scalar())).collect();
            json!({ "enum": values })
        }
        6 => json!({ "type": "string", "minLength": small_length(tc) }),
        7 => json!({ "type": "string", "maxLength": small_length(tc) }),
        8 => {
            let (min, max) = ordered(small_length(tc), small_length(tc));
            json!({ "type": "string", "minLength": min, "maxLength": max })
        }
        9 => {
            let pattern = tc.draw(gs::sampled_from(vec!["^a", "b$", "[0-9]+", "x"]));
            json!({ "type": "string", "pattern": pattern })
        }
        10 => json!({ "type": "string", "minLength": small_length(tc), "pattern": "^a" }),
        11 => json!({ "type": "integer", "minimum": small_int(tc) }),
        12 => json!({ "type": "integer", "maximum": small_int(tc) }),
        13 => {
            let (min, max) = ordered(small_int(tc), small_int(tc));
            json!({ "type": "integer", "minimum": min, "maximum": max })
        }
        // Draft 6+ spells exclusivity as a number; Draft 4 as a boolean modifier. Each is meta-invalid
        // under the other dialect, where the drawn document is simply rejected before modeling.
        14 => json!({ "type": "integer", "exclusiveMinimum": small_int(tc) }),
        15 => json!({ "type": "integer", "exclusiveMaximum": small_int(tc) }),
        16 => {
            json!({ "type": "integer", "minimum": small_int(tc), "exclusiveMinimum": tc.draw(gs::booleans()) })
        }
        17 => {
            json!({ "type": "integer", "maximum": small_int(tc), "exclusiveMaximum": tc.draw(gs::booleans()) })
        }
        18 => json!({ "type": "object", "minProperties": small_length(tc) }),
        19 => json!({ "type": "object", "maxProperties": small_length(tc) }),
        20 => {
            let (min, max) = ordered(small_length(tc), small_length(tc));
            json!({ "type": "object", "minProperties": min, "maxProperties": max })
        }
        21 => json!({ "type": "array", "minItems": small_length(tc) }),
        22 => json!({ "type": "array", "maxItems": small_length(tc) }),
        23 => {
            let (min, max) = ordered(small_length(tc), small_length(tc));
            json!({ "type": "array", "minItems": min, "maxItems": max })
        }
        24 => json!({ "type": "object", "required": draw_keys(tc) }),
        25 => {
            json!({ "type": "object", "required": draw_keys(tc), "maxProperties": small_length(tc) })
        }
        26 => json!({ "type": "array", "uniqueItems": tc.draw(gs::booleans()) }),
        27 => {
            json!({ "type": "array", "uniqueItems": true, "maxItems": small_length(tc) })
        }
        28 => json!({ "type": "number", "multipleOf": divisor(tc) }),
        29 => json!({ "multipleOf": divisor(tc) }),
        30 => json!({ "type": "integer", "multipleOf": divisor(tc) }),
        31 => {
            let (min, max) = ordered(small_int(tc), small_int(tc));
            json!({ "type": "number", "minimum": min, "maximum": max, "multipleOf": divisor(tc) })
        }
        32 => json!({ "type": "object", "propertyNames": { "maxLength": small_length(tc) } }),
        33 => {
            let keys = draw_keys(tc);
            json!({ "type": "object", "propertyNames": { "enum": keys } })
        }
        34 => json!({ "type": "object", "properties": { "a": { "type": draw_type(tc) } } }),
        35 => {
            json!({ "type": "object", "properties": { "a": true, "b": { "type": "integer" } } })
        }
        36 => json!({ "type": "object", "properties": { "a": false } }),
        37 => {
            json!({ "type": "object", "properties": { "a": { "type": "string", "format": "email" } } })
        }
        38 => {
            json!({ "type": "object", "properties": { "a": { "type": "string", "format": "unknown-fmt" } } })
        }
        // Object-valued members collide with the property leaves above, where Draft 4 aliases the
        // nested number spellings apart.
        39 => json!({ "enum": [{ "a": small_int(tc) }] }),
        40 => json!({ "const": { "a": tc.draw(arbitrary_scalar()) } }),
        41 => json!({ "type": "array", "items": { "type": draw_type(tc) } }),
        42 => json!({ "type": "array", "items": false }),
        43 => {
            json!({ "type": "array", "items": { "type": "string", "format": "unknown-fmt" } })
        }
        // Array-valued members collide with the item leaves above.
        44 => json!({ "enum": [[tc.draw(arbitrary_scalar())]] }),
        45 => json!({ "type": "object", "patternProperties": { "^a": { "type": draw_type(tc) } } }),
        // The pattern reaches a named key, so the two schemas fold together on it.
        46 => json!({
            "type": "object",
            "properties": { "ab": { "type": "string" } },
            "patternProperties": { "^a": { "type": "string", "minLength": small_length(tc) } }
        }),
        47 => json!({ "type": "object", "patternProperties": { "^a": false } }),
        // A finite key set leaves no key outside it, so the patterns move onto the keys they match.
        48 => {
            let keys = draw_keys(tc);
            json!({
                "type": "object",
                "propertyNames": { "enum": keys },
                "patternProperties": { "^a": { "type": "integer" } }
            })
        }
        49 => json!({
            "type": "object",
            "patternProperties": { "^a": { "type": "string", "format": "unknown-fmt" } }
        }),
        // An `integer` draw declines the complement, so both negate outcomes stay exercised.
        50 => json!({ "not": { "type": draw_type(tc) } }),
        51 => json!({ "not": { "enum": [false, true] } }),
        52 => json!({ "type": "array", "contains": { "type": draw_type(tc) } }),
        // Drafts before 2019-09 ignore the count window keywords as unknown.
        53 => {
            let (min, max) = ordered(small_length(tc), small_length(tc));
            json!({ "contains": { "type": draw_type(tc) }, "minContains": min, "maxContains": max })
        }
        54 => json!({ "type": "number", "minimum": small_int(tc) }),
        55 => json!({ "type": "number", "maximum": small_int(tc) }),
        56 => {
            let (min, max) = ordered(small_int(tc), small_int(tc));
            json!({ "type": "number", "minimum": min, "maximum": max })
        }
        // Meta-invalid under Draft 4, where the drawn document is simply rejected before modeling.
        57 => json!({ "type": "number", "exclusiveMinimum": small_int(tc) }),
        // Overlapping branches exercise the exactly-one encoding; disjoint draws its fast path.
        58 => json!({ "oneOf": [{ "type": "string" }, { "minLength": 1 }] }),
        59 => json!({ "oneOf": [
            { "const": small_int(tc) },
            { "enum": [small_int(tc), small_int(tc)] }
        ] }),
        _ => json!({ "type": ["string", "integer"] }),
    }
}

// Keys drawn from a small pool so different leaves overlap often enough to exercise merging.
fn draw_keys(tc: &TestCase) -> Vec<&'static str> {
    let count = tc.draw(gs::integers::<usize>().min_value(0).max_value(2));
    let mut keys: Vec<&'static str> = (0..count)
        .map(|_| tc.draw(gs::sampled_from(vec!["a", "b", "c", "ab"])))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn draw_schema(tc: &TestCase, depth: u32) -> Value {
    if depth == 0 || tc.draw(gs::booleans()) {
        return draw_leaf(tc);
    }
    let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(2));
    let branches: Vec<Value> = (0..count).map(|_| draw_schema(tc, depth - 1)).collect();
    if tc.draw(gs::booleans()) {
        json!({ "allOf": branches })
    } else {
        json!({ "anyOf": branches })
    }
}

// Meta-valid keywords the canonicaliser does not model; a document carrying one stays `Raw`.
fn draw_unmodeled_leaf(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(4)) {
        0 => json!({ "additionalProperties": { "type": "integer" } }),
        1 => json!({ "not": { "pattern": "^a" } }),
        2 => json!({ "$defs": { "a": { "type": "null" } }, "$ref": "#/$defs/a" }),
        3 => json!({ "format": "email" }),
        _ => json!({ "oneOf": [{ "type": "string" }, { "minLength": 1 }] }),
    }
}

fn draw_broad_schema(tc: &TestCase, depth: u32) -> Value {
    if depth == 0 || tc.draw(gs::booleans()) {
        return if tc.draw(gs::booleans()) {
            draw_leaf(tc)
        } else {
            draw_unmodeled_leaf(tc)
        };
    }
    let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(2));
    let branches: Vec<Value> = (0..count)
        .map(|_| draw_broad_schema(tc, depth - 1))
        .collect();
    if tc.draw(gs::booleans()) {
        json!({ "allOf": branches })
    } else {
        json!({ "anyOf": branches })
    }
}

fn canonicalize_with_formats(
    schema: &Value,
    draft: Draft,
    validate_formats: bool,
) -> Option<Value> {
    jsonschema::canonical::options()
        .with_draft(draft)
        .should_validate_formats(validate_formats)
        .canonicalize(schema)
        .ok()
        .map(|canonical| canonical.to_json_schema())
}

fn canonicalize(schema: &Value, draft: Draft) -> Option<Value> {
    canonicalize_with_formats(schema, draft, false)
}

// Canonicalizing an already-canonical form yields the same form.
#[hegel::test(test_cases = 5_000)]
fn canonicalize_is_idempotent(tc: TestCase) {
    let draft = draw_draft(&tc);
    let validate_formats = tc.draw(gs::booleans());
    let schema = draw_schema(&tc, 3);
    if let Some(once) = canonicalize_with_formats(&schema, draft, validate_formats) {
        let twice = canonicalize_with_formats(&once, draft, validate_formats)
            .expect("a canonical form re-canonicalizes");
        assert_eq!(once, twice);
    }
}

// The canonical form accepts exactly the values the original does, across drafts.
#[hegel::test(test_cases = 5_000)]
fn canonical_form_preserves_validation(tc: TestCase) {
    let draft = draw_draft(&tc);
    let validate_formats = tc.draw(gs::booleans());
    let schema = draw_schema(&tc, 3);
    let instance = tc.draw(arbitrary_instance());
    let Some(emitted) = canonicalize_with_formats(&schema, draft, validate_formats) else {
        return;
    };
    let build = |value: &Value| {
        jsonschema::options()
            .with_draft(draft)
            .should_validate_formats(validate_formats)
            .build(value)
    };
    let (Ok(raw), Ok(canonical)) = (build(&schema), build(&emitted)) else {
        return;
    };
    assert_eq!(
        raw.is_valid(&instance),
        canonical.is_valid(&instance),
        "{schema} vs {emitted} on {instance}"
    );
}

// `not not s` accepts exactly what `s` accepts, so when the double complement is modeled both
// spellings land on one canonical form. A raw result round-trips the document verbatim and
// carries no claim to check.
#[hegel::test(test_cases = 5_000)]
fn double_negation_converges(tc: TestCase) {
    let draft = draw_draft(&tc);
    let schema = draw_schema(&tc, 2);
    let doubled = json!({ "not": { "not": schema } });
    let Some(via_double) = canonicalize(&doubled, draft) else {
        return;
    };
    if via_double == doubled {
        return;
    }
    let direct =
        canonicalize(&schema, draft).expect("a modeled double complement implies a modeled child");
    assert_eq!(direct, via_double, "{schema}");
}

// A value set intersected with an integer bound preserves validation on its own members and their
// float spellings - the interaction that a dropped Draft 4 integer guard makes unsound.
#[hegel::test(test_cases = 5_000)]
fn integer_value_set_intersection_preserves_validation(tc: TestCase) {
    let draft = draw_draft(&tc);
    let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(3));
    let members: Vec<i32> = (0..count).map(|_| small_int(&tc)).collect();
    let (min, max) = ordered(small_int(&tc), small_int(&tc));
    let schema = json!({
        "allOf": [
            { "enum": members },
            { "type": "integer", "minimum": min, "maximum": max },
        ]
    });
    // An instance drawn from the members, spelled as an integer or as an integer-valued float.
    let chosen = members[tc.draw(gs::integers::<usize>().min_value(0).max_value(count - 1))];
    let instance = if tc.draw(gs::booleans()) {
        json!(chosen)
    } else {
        json!(f64::from(chosen))
    };
    let Some(emitted) = canonicalize(&schema, draft) else {
        return;
    };
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let (Ok(raw), Ok(canonical)) = (build(&schema), build(&emitted)) else {
        return;
    };
    assert_eq!(raw.is_valid(&instance), canonical.is_valid(&instance));
}

// An object value set beside per-property schemas preserves validation on its members and the
// float spellings of their numbers - the interaction a dropped Draft 4 guard makes unsound.
#[hegel::test(test_cases = 5_000)]
fn object_member_intersection_preserves_validation(tc: TestCase) {
    let draft = draw_draft(&tc);
    let chosen = small_int(&tc);
    let child = tc.draw(gs::sampled_from(vec!["integer", "number", "string"]));
    let branches = vec![
        json!({ "enum": [{ "a": chosen }] }),
        json!({ "type": "object", "properties": { "a": { "type": child } } }),
    ];
    let schema = if tc.draw(gs::booleans()) {
        json!({ "allOf": branches })
    } else {
        json!({ "anyOf": branches })
    };
    // The member itself, spelled with the integer or its float alias.
    let instance = if tc.draw(gs::booleans()) {
        json!({ "a": chosen })
    } else {
        json!({ "a": f64::from(chosen) })
    };
    let Some(emitted) = canonicalize(&schema, draft) else {
        return;
    };
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let (Ok(raw), Ok(canonical)) = (build(&schema), build(&emitted)) else {
        return;
    };
    assert_eq!(
        raw.is_valid(&instance),
        canonical.is_valid(&instance),
        "{schema} vs {emitted} on {instance}"
    );
}

// Any input reduces to `Ok(modeled)`, `Ok(Raw)`, or an error - never a panic.
#[hegel::test(test_cases = 5_000)]
fn canonicalize_never_panics(tc: TestCase) {
    let draft = draw_draft(&tc);
    let schema = if tc.draw(gs::booleans()) {
        tc.draw(json_gs::values())
    } else {
        draw_broad_schema(&tc, 3)
    };
    let _ = jsonschema::canonical::options()
        .with_draft(draft)
        .canonicalize(&schema);
}

// Divisors combined under `allOf`/`anyOf` keep validation, including on the integers where the
// arithmetic the validator picks per spelling starts to disagree with exact rationals.
#[hegel::test(test_cases = 5_000)]
fn divisor_algebra_preserves_validation(tc: TestCase) {
    let draft = draw_draft(&tc);
    let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(3));
    let branches: Vec<Value> = (0..count)
        .map(|_| {
            match tc.draw(gs::integers::<u8>().min_value(0).max_value(4)) {
                0 => json!({ "type": "number", "multipleOf": divisor(&tc) }),
                1 => json!({ "type": "integer", "multipleOf": divisor(&tc) }),
                2 => json!({ "multipleOf": divisor(&tc) }),
                // A value set beside a divisor: membership is decided by the same arithmetic.
                3 => json!({ "const": wide_number(&tc) }),
                _ => json!({ "enum": [wide_number(&tc), tc.draw(arbitrary_scalar())] }),
            }
        })
        .collect();
    let schema = if tc.draw(gs::booleans()) {
        json!({ "allOf": branches })
    } else {
        json!({ "anyOf": branches })
    };
    let instance = if tc.draw(gs::booleans()) {
        wide_number(&tc)
    } else {
        tc.draw(arbitrary_instance())
    };
    let Some(emitted) = canonicalize(&schema, draft) else {
        return;
    };
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let (Ok(raw), Ok(canonical)) = (build(&schema), build(&emitted)) else {
        return;
    };
    assert_eq!(
        raw.is_valid(&instance),
        canonical.is_valid(&instance),
        "{schema} vs {emitted} on {instance}"
    );
}

// The order divisors arrive in is not part of the schema's meaning, so it cannot change the form.
// An unmodeled document is kept as written, so only modeled ones carry the claim.
#[hegel::test(test_cases = 5_000)]
fn divisor_order_does_not_change_the_form(tc: TestCase) {
    let draft = draw_draft(&tc);
    let count = tc.draw(gs::integers::<usize>().min_value(2).max_value(4));
    let branches: Vec<Value> = (0..count)
        .map(|_| json!({ "type": "number", "multipleOf": divisor(&tc) }))
        .collect();
    let reversed: Vec<Value> = branches.iter().rev().cloned().collect();
    let schema = json!({ "allOf": branches });
    let Ok(canonical) = jsonschema::canonical::options()
        .with_draft(draft)
        .canonicalize(&schema)
    else {
        return;
    };
    if canonical.kind() == jsonschema::canonical::CanonicalKind::Raw {
        return;
    }
    assert_eq!(
        Some(canonical.to_json_schema()),
        canonicalize(&json!({ "allOf": reversed }), draft),
        "{schema}"
    );
}

// A divisor every other one already covers adds nothing, so the form cannot notice it.
#[hegel::test(test_cases = 5_000)]
fn a_redundant_divisor_does_not_change_the_form(tc: TestCase) {
    let draft = draw_draft(&tc);
    let left = tc.draw(gs::integers::<u32>().min_value(1).max_value(64));
    let right = tc.draw(gs::integers::<u32>().min_value(1).max_value(64));
    let mut common = (left, right);
    while common.1 != 0 {
        common = (common.1, common.0 % common.1);
    }
    let pair = json!([
        { "type": "number", "multipleOf": left },
        { "type": "number", "multipleOf": right }
    ]);
    let with_common = json!([
        { "type": "number", "multipleOf": left },
        { "type": "number", "multipleOf": right },
        { "type": "number", "multipleOf": common.0 }
    ]);
    assert_eq!(
        canonicalize(&json!({ "allOf": pair }), draft),
        canonicalize(&json!({ "allOf": with_common }), draft),
        "gcd({left}, {right}) = {}",
        common.0
    );
}

// Equality-preserving syntactic rewrites: each keeps the accepted value set unchanged, so the
// canonical forms must be IR-equal.
fn rewrite_schema(tc: &TestCase, schema: &Value) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(4)) {
        0 => json!({ "allOf": [schema] }),
        // The empty conjunct says nothing; unlike `true` it is meta-valid in every draft.
        1 => json!({ "allOf": [schema, {}] }),
        // A union does not change when one branch appears twice.
        2 => match schema.get("anyOf").and_then(Value::as_array) {
            Some(branches) if !branches.is_empty() => {
                let mut extended = branches.clone();
                extended.push(branches[0].clone());
                let mut rewritten = schema
                    .as_object()
                    .expect("`anyOf` sits in an object")
                    .clone();
                rewritten.insert("anyOf".to_string(), Value::Array(extended));
                Value::Object(rewritten)
            }
            _ => json!({ "anyOf": [schema] }),
        },
        // A union does not change when its branches are reordered.
        3 => match schema.get("anyOf").and_then(Value::as_array) {
            Some(branches) if branches.len() >= 2 => {
                let mut rotated = branches.clone();
                rotated.rotate_left(1);
                let mut rewritten = schema
                    .as_object()
                    .expect("`anyOf` sits in an object")
                    .clone();
                rewritten.insert("anyOf".to_string(), Value::Array(rotated));
                Value::Object(rewritten)
            }
            _ => json!({ "anyOf": [schema] }),
        },
        // A lone `type: [a, b]` admits the same values as the union of its single-type spellings.
        _ => match (
            schema.as_object(),
            schema.get("type").and_then(Value::as_array),
        ) {
            (Some(object), Some(names)) if object.len() == 1 && names.len() >= 2 => json!({
                "anyOf": names
                    .iter()
                    .map(|name| json!({ "type": name }))
                    .collect::<Vec<_>>()
            }),
            _ => json!({ "allOf": [schema] }),
        },
    }
}

#[hegel::test(test_cases = 5_000)]
fn equality_preserving_rewrites_converge(tc: TestCase) {
    let draft = draw_draft(&tc);
    let schema = draw_schema(&tc, 2);
    // Draft 4's metaschema rejects boolean subschemas, so a wrap of a boolean root is not a
    // meta-valid document there.
    if !schema.is_object() {
        return;
    }
    let rewritten = rewrite_schema(&tc, &schema);
    let Ok(original) = jsonschema::canonical::options()
        .with_draft(draft)
        .canonicalize(&schema)
    else {
        return;
    };
    // A raw document round-trips verbatim, so a wrapper changes it by construction.
    if matches!(original.view(), CanonicalView::Raw(_)) {
        return;
    }
    let converged = jsonschema::canonical::options()
        .with_draft(draft)
        .canonicalize(&rewritten)
        .expect("a rewrite of a canonicalizable schema canonicalizes");
    assert_eq!(
        original, converged,
        "schema = {schema}\n  rewritten = {rewritten}"
    );
}

// The canonical complement rejects exactly what the schema accepts; the runtime validator is the
// independent ground truth. A raw result round-trips the document verbatim and carries no claim.
#[hegel::test(test_cases = 5_000)]
fn negation_complements_the_validator_verdict(tc: TestCase) {
    let draft = draw_draft(&tc);
    let validate_formats = tc.draw(gs::booleans());
    let schema = draw_schema(&tc, 2);
    let negated = json!({ "not": schema });
    let Some(emitted) = canonicalize_with_formats(&negated, draft, validate_formats) else {
        return;
    };
    if emitted == negated {
        return;
    }
    // Random instances almost never land on a window's limit, so half the draws reuse a numeric
    // literal from the schema itself - the boundary is where a flipped inclusivity hides.
    let literals = numeric_literals(&schema);
    let instance = if !literals.is_empty() && tc.draw(gs::booleans()) {
        tc.draw(gs::sampled_from(literals))
    } else {
        tc.draw(arbitrary_instance())
    };
    let build = |value: &Value| {
        jsonschema::options()
            .with_draft(draft)
            .should_validate_formats(validate_formats)
            .build(value)
    };
    let (Ok(raw), Ok(canonical)) = (build(&schema), build(&emitted)) else {
        return;
    };
    assert_eq!(
        raw.is_valid(&instance),
        !canonical.is_valid(&instance),
        "schema = {schema}\n  complement = {emitted}\n  instance = {instance}"
    );
}

fn numeric_literals(schema: &Value) -> Vec<Value> {
    match schema {
        Value::Number(_) => vec![schema.clone()],
        Value::Array(items) => items.iter().flat_map(numeric_literals).collect(),
        Value::Object(map) => map.values().flat_map(numeric_literals).collect(),
        Value::Null | Value::Bool(_) | Value::String(_) => Vec::new(),
    }
}
