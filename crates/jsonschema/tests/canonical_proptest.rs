#![cfg(not(target_arch = "wasm32"))]
use hegel::{extras::serde_json as json_gs, generators as gs, TestCase};
use jsonschema::Draft;
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
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(7)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => json!(tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))),
        3 => json!(finite_float(&tc)),
        // An integer-valued float (`2.0`): Draft 4 treats it as a non-integer, later drafts as an integer.
        4 => json!(f64::from(
            tc.draw(gs::integers::<i32>().min_value(-4).max_value(4))
        )),
        5 => Value::String(tc.draw(gs::text().max_size(5))),
        6 => json!([]),
        _ => json!({}),
    }
}

// A modeled leaf: value sets, type sets, string facets, integer interval bounds, and container sizes.
fn draw_leaf(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(24)) {
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
        _ => json!({ "type": ["string", "integer"] }),
    }
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
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(8)) {
        0 => {
            json!({ "type": "integer", "multipleOf": tc.draw(gs::integers::<u8>().min_value(1).max_value(7)) })
        }
        1 => json!({ "type": "object", "required": ["a"] }),
        2 => json!({ "type": "object", "properties": { "a": { "type": "integer" } } }),
        3 => json!({ "type": "array", "items": { "type": "integer" } }),
        4 => json!({ "type": "array", "uniqueItems": true }),
        5 => json!({ "not": { "type": "string" } }),
        6 => json!({ "$defs": { "a": { "type": "null" } }, "$ref": "#/$defs/a" }),
        7 => json!({ "format": "email" }),
        _ => json!({ "oneOf": [{ "type": "string" }, { "type": "integer" }] }),
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

fn canonicalize(schema: &Value, draft: Draft) -> Option<Value> {
    jsonschema::canonical::options()
        .with_draft(draft)
        .canonicalize(schema)
        .ok()
        .map(|canonical| canonical.to_json_schema())
}

// Canonicalizing an already-canonical form yields the same form.
#[hegel::test(test_cases = 10_000)]
fn canonicalize_is_idempotent(tc: TestCase) {
    let draft = draw_draft(&tc);
    let schema = draw_schema(&tc, 3);
    if let Some(once) = canonicalize(&schema, draft) {
        let twice = canonicalize(&once, draft).expect("a canonical form re-canonicalizes");
        assert_eq!(once, twice);
    }
}

// The canonical form accepts exactly the values the original does, across drafts.
#[hegel::test(test_cases = 10_000)]
fn canonical_form_preserves_validation(tc: TestCase) {
    let draft = draw_draft(&tc);
    let schema = draw_schema(&tc, 3);
    let instance = tc.draw(arbitrary_instance());
    let Some(emitted) = canonicalize(&schema, draft) else {
        return;
    };
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let (Ok(raw), Ok(canonical)) = (build(&schema), build(&emitted)) else {
        return;
    };
    assert_eq!(raw.is_valid(&instance), canonical.is_valid(&instance));
}

// A value set intersected with an integer bound preserves validation on its own members and their
// float spellings - the interaction that a dropped Draft 4 integer guard makes unsound.
#[hegel::test(test_cases = 10_000)]
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

// Any input reduces to `Ok(modeled)`, `Ok(Raw)`, or an error - never a panic.
#[hegel::test(test_cases = 10_000)]
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
