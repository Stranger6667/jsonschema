#![cfg(not(target_arch = "wasm32"))]
use hegel::{extras::serde_json as json_gs, generators as gs, TestCase};
use jsonschema::Draft;
use serde_json::{json, Value};

const DRAFT: Draft = Draft::Draft202012;

fn draw_type(tc: &TestCase) -> &'static str {
    tc.draw(gs::sampled_from(vec![
        "null", "boolean", "integer", "number", "string", "array", "object",
    ]))
}

fn small_length(tc: &TestCase) -> u8 {
    tc.draw(gs::integers::<u8>().min_value(0).max_value(4))
}

fn ordered(a: u8, b: u8) -> (u8, u8) {
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
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(6)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => json!(tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))),
        3 => json!(finite_float(&tc)),
        4 => Value::String(tc.draw(gs::text().max_size(5))),
        5 => json!([]),
        _ => json!({}),
    }
}

// A modeled leaf: value sets, type sets, and string facets.
fn draw_leaf(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(11)) {
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

// Meta-valid keywords PR6 does not model; a document carrying one stays `Raw`.
fn draw_unmodeled_leaf(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(9)) {
        0 => {
            json!({ "type": "integer", "minimum": tc.draw(gs::integers::<i32>().min_value(-8).max_value(8)) })
        }
        1 => {
            json!({ "type": "integer", "multipleOf": tc.draw(gs::integers::<u8>().min_value(1).max_value(7)) })
        }
        2 => json!({ "type": "object", "required": ["a"] }),
        3 => json!({ "type": "object", "properties": { "a": { "type": "integer" } } }),
        4 => json!({ "type": "array", "items": { "type": "integer" } }),
        5 => json!({ "type": "array", "uniqueItems": true }),
        6 => json!({ "not": { "type": "string" } }),
        7 => json!({ "$defs": { "a": { "type": "null" } }, "$ref": "#/$defs/a" }),
        8 => json!({ "format": "email" }),
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

fn canonicalize(schema: &Value) -> Option<Value> {
    jsonschema::canonical::options()
        .with_draft(DRAFT)
        .canonicalize(schema)
        .ok()
        .map(|canonical| canonical.to_json_schema())
}

// Canonicalizing an already-canonical form yields the same form.
#[hegel::test(test_cases = 10_000)]
fn canonicalize_is_idempotent(tc: TestCase) {
    let schema = draw_schema(&tc, 3);
    if let Some(once) = canonicalize(&schema) {
        let twice = canonicalize(&once).expect("a canonical form re-canonicalizes");
        assert_eq!(once, twice);
    }
}

// The canonical form accepts exactly the values the original does.
#[hegel::test(test_cases = 10_000)]
fn canonical_form_preserves_validation(tc: TestCase) {
    let schema = draw_schema(&tc, 3);
    let instance = tc.draw(arbitrary_instance());
    let Some(emitted) = canonicalize(&schema) else {
        return;
    };
    let build = |value: &Value| jsonschema::options().with_draft(DRAFT).build(value);
    let (Ok(raw), Ok(canonical)) = (build(&schema), build(&emitted)) else {
        return;
    };
    assert_eq!(raw.is_valid(&instance), canonical.is_valid(&instance));
}

// Any input reduces to `Ok(modeled)`, `Ok(Raw)`, or an error - never a panic.
#[hegel::test(test_cases = 10_000)]
fn canonicalize_never_panics(tc: TestCase) {
    let schema = if tc.draw(gs::booleans()) {
        tc.draw(json_gs::values())
    } else {
        draw_broad_schema(&tc, 3)
    };
    let _ = jsonschema::canonical::options()
        .with_draft(DRAFT)
        .canonicalize(&schema);
}
