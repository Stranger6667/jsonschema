#![cfg(not(target_arch = "wasm32"))]
use hegel::{extras::serde_json as json_gs, generators as gs, TestCase};
use jsonschema::{
    canonical::{CanonicalSchema, CanonicalView, Containment, Satisfiability},
    Draft, JsonType,
};
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

/// A count keyword's value: small enough to reason about, or one of the ends a machine has to hold
/// without building an instance that long.
fn drawn_length(tc: &TestCase) -> u64 {
    if tc.draw(gs::integers::<u8>().min_value(0).max_value(15)) == 0 {
        return tc.draw(gs::sampled_from(vec![
            u64::from(u32::MAX),
            u64::MAX - 1,
            u64::MAX,
            1_000_000_000_000,
        ]));
    }
    u64::from(small_length(tc))
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
fn arbitrary_scalar(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(5)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => Value::String(tc.draw(gs::text().max_size(3))),
        3 => json!(tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))),
        4 => json!(finite_float(tc)),
        _ => aliased_number(tc),
    }
}

#[hegel::composite]
fn arbitrary_instance(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(10)) {
        0 => Value::Null,
        1 => Value::Bool(tc.draw(gs::booleans())),
        2 => json!(tc.draw(gs::integers::<i32>().min_value(-8).max_value(8))),
        3 => json!(finite_float(tc)),
        // An integer-valued float (`2.0`): Draft 4 treats it as a non-integer, later drafts as an integer.
        4 => json!(f64::from(
            tc.draw(gs::integers::<i32>().min_value(-4).max_value(4))
        )),
        5 => Value::String(tc.draw(gs::text().max_size(5))),
        6 => wide_number(tc),
        7 => json!([]),
        8 => {
            let mut object = serde_json::Map::new();
            for key in draw_keys(tc) {
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

// `^$` matches only the empty string, the one value `maxLength: 0` leaves.
fn draw_pattern(tc: &TestCase) -> &'static str {
    tc.draw(gs::sampled_from(vec!["^a", "b$", "[0-9]+", "x", "^$"]))
}

fn draw_format(tc: &TestCase) -> &'static str {
    tc.draw(gs::sampled_from(vec![
        "email",
        "date",
        "uuid",
        "unknown-fmt",
    ]))
}

// Key patterns that overlap each other and the drawn key pool, so a shield and a name constraint
// can each hold what the other turns away.
fn draw_key_pattern(tc: &TestCase) -> &'static str {
    tc.draw(gs::sampled_from(vec!["^a", "b$", "^x", "^[ab]$", "^a.*"]))
}

// A modeled leaf: value sets, type sets, string facets, integer interval bounds, and container sizes.
fn draw_leaf(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(105)) {
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
        6 => json!({ "type": "string", "minLength": drawn_length(tc) }),
        7 => json!({ "type": "string", "maxLength": drawn_length(tc) }),
        8 => {
            let (min, max) = ordered(small_length(tc), small_length(tc));
            json!({ "type": "string", "minLength": min, "maxLength": max })
        }
        9 => json!({ "type": "string", "pattern": draw_pattern(tc) }),
        10 => json!({ "type": "string", "minLength": drawn_length(tc), "pattern": "^a" }),
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
        21 => json!({ "type": "array", "minItems": drawn_length(tc) }),
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
        // `then` alone needs the condition's complement; `else` alone needs none of it.
        60 => json!({ "if": { "type": draw_type(tc) }, "then": { "type": draw_type(tc) } }),
        61 => json!({ "if": { "type": draw_type(tc) }, "else": { "type": draw_type(tc) } }),
        62 => json!({
            "if": { "type": draw_type(tc) },
            "then": { "type": draw_type(tc) },
            "else": { "type": draw_type(tc) }
        }),
        // Trigger keys drawn from the shared pool collide with the object leaves above.
        64 => {
            json!({ "dependencies": { tc.draw(gs::sampled_from(vec!["a", "b"])): draw_keys(tc) } })
        }
        65 => json!({ "dependencies": { "a": { "required": draw_keys(tc) } } }),
        66 => {
            json!({ "dependentRequired": { tc.draw(gs::sampled_from(vec!["a", "c"])): draw_keys(tc) } })
        }
        67 => {
            json!({ "dependentSchemas": { "a": { "properties": { "b": { "type": draw_type(tc) } } } } })
        }
        // Closed maps: the keys collide with the object leaves above; an unconstrained entry
        // exercises the entry the map normalization drops.
        68 => json!({
            "type": "object",
            "properties": { "a": {}, "b": { "type": draw_type(tc) } },
            "additionalProperties": false
        }),
        69 => json!({
            "type": "object",
            "properties": { "a": { "type": draw_type(tc) } },
            "required": ["a"],
            "additionalProperties": false
        }),
        // Shields: unnamed keys answer to the schema, so these collide with every object leaf
        // above; the named entry exercises the crossing in intersect.
        70 => json!({ "type": "object", "additionalProperties": { "type": draw_type(tc) } }),
        71 => json!({
            "type": "object",
            "properties": { "a": { "type": draw_type(tc) } },
            "additionalProperties": { "type": draw_type(tc) }
        }),
        72 => json!({ "type": "string", "contentMediaType": "application/json" }),
        73 => json!({ "type": "string", "contentEncoding": "base64" }),
        // Same object composes decode-then-check, which this leaf's independent facets cannot spell.
        74 => json!({
            "type": "string",
            "contentMediaType": "application/json",
            "contentEncoding": "base64"
        }),
        // `unevaluated*` with no in-place applicator beside it degrades to its `additional*` twin.
        75 => {
            json!({ "type": "object", "properties": { "a": true }, "unevaluatedProperties": false })
        }
        76 => {
            json!({ "type": "array", "prefixItems": [{ "type": "integer" }], "unevaluatedItems": false })
        }
        77 => json!({ "type": "object", "unevaluatedProperties": { "type": draw_type(tc) } }),
        // A pattern matching finitely many keys names them, which frees the `additionalProperties`
        // pairing the unbounded spellings above keep raw.
        78 => {
            json!({ "type": "object", "patternProperties": { "^a$": { "type": draw_type(tc) } }, "additionalProperties": { "type": draw_type(tc) } })
        }
        79 => {
            json!({ "type": "object", "patternProperties": { "^(a|b)$": { "type": draw_type(tc) } }, "properties": { "a": { "type": draw_type(tc) } } })
        }
        80 => {
            json!({ "type": "object", "patternProperties": { "^a$": { "type": draw_type(tc) } }, "additionalProperties": false })
        }
        // An `unevaluatedProperties` beside `allOf` degrades over the branches' hoisted names.
        81 => {
            json!({ "type": "object", "allOf": [{ "properties": { "a": { "type": draw_type(tc) } } }], "unevaluatedProperties": false })
        }
        82 => {
            json!({ "type": "object", "allOf": [{ "properties": { "a": true } }, { "properties": { "b": true } }], "unevaluatedProperties": { "type": draw_type(tc) } })
        }
        83 => {
            json!({ "type": "object", "allOf": [{ "patternProperties": { "^a": true } }], "properties": { "b": true }, "unevaluatedProperties": false })
        }
        // An `unevaluatedItems` beside `allOf` degrades over the branches' longest tuple.
        84 => {
            json!({ "type": "array", "allOf": [{ "prefixItems": [{ "type": draw_type(tc) }] }], "unevaluatedItems": false })
        }
        85 => {
            json!({ "type": "array", "prefixItems": [{ "type": draw_type(tc) }], "allOf": [{ "prefixItems": [true, { "type": draw_type(tc) }] }], "unevaluatedItems": false })
        }
        86 => {
            json!({ "type": "array", "allOf": [{ "items": [{ "type": draw_type(tc) }] }], "unevaluatedItems": { "type": draw_type(tc) } })
        }
        // Positional elements: 2020-12 spells the tuple `prefixItems` and the drafts before it
        // spell it as an array-form `items`, each meta-invalid where the other one is the spelling.
        87 => json!({ "type": "array", "prefixItems": [{ "type": draw_type(tc) }] }),
        88 => {
            json!({ "type": "array", "items": [{ "type": draw_type(tc) }, { "type": draw_type(tc) }] })
        }
        89 => {
            json!({ "type": "array", "items": [{ "type": draw_type(tc) }], "additionalItems": false })
        }
        // A tail past a tuple, which the complement has no branch for.
        90 => {
            json!({ "type": "array", "prefixItems": [{ "type": draw_type(tc) }], "items": { "type": draw_type(tc) } })
        }
        91 => {
            json!({ "type": "array", "items": [{ "type": draw_type(tc) }], "additionalItems": { "type": draw_type(tc) } })
        }
        63 => {
            let (first, second) = (draw_type(tc), draw_type(tc));
            let types = if first == second {
                vec![first]
            } else {
                vec![first, second]
            };
            json!({ "type": types })
        }
        // A repeat demand: the dual of distinctness, which only a complement spells.
        92 => json!({ "not": { "type": "array", "uniqueItems": true } }),
        93 => {
            json!({ "type": "array", "allOf": [{ "not": { "type": "array", "uniqueItems": true } }] })
        }
        // A no-op unevaluated*: rejects nothing regardless of what else is on the object.
        94 => json!({ "type": "object", "unevaluatedProperties": true }),
        95 => json!({ "type": "array", "unevaluatedItems": true }),
        96 => json!({
            "type": "object",
            "patternProperties": { draw_key_pattern(tc): { "type": draw_type(tc) } },
            "additionalProperties": false
        }),
        97 => json!({ "type": "object", "propertyNames": { "pattern": draw_key_pattern(tc) } }),
        98 => json!({
            "not": {
                "type": "object",
                "propertyNames": { "pattern": draw_key_pattern(tc) },
                "required": draw_keys(tc)
            }
        }),
        99 => json!({
            "type": "object",
            "additionalProperties": false,
            "maxProperties": small_length(tc),
            "patternProperties": {
                draw_key_pattern(tc): { "propertyNames": { "pattern": draw_key_pattern(tc) } }
            },
            "properties": { "a": { "maxProperties": small_length(tc) } }
        }),
        // A length limit beside a keyword no length can answer: `maxLength: 0` leaves the empty
        // string alone, which the `pattern` or `format` beside it either accepts or rejects.
        100 => json!({
            "type": "string",
            "maxLength": small_length(tc),
            "pattern": draw_pattern(tc)
        }),
        101 => {
            json!({ "type": "string", "maxLength": small_length(tc), "format": draw_format(tc) })
        }
        102 => {
            let (min, max) = ordered(small_length(tc), small_length(tc));
            json!({ "type": "string", "minLength": min, "maxLength": max, "pattern": draw_pattern(tc) })
        }
        103 => json!({
            "type": "string",
            "maxLength": small_length(tc),
            "contentEncoding": "base64"
        }),
        // An element schema written as a complement: under Draft 4 its negation takes a whole number
        // that its own `type: integer` reading of the same value refuses.
        104 => json!({ "type": "array", "items": { "not": { "type": draw_type(tc) } } }),
        // A one-element array member beside a demand on that element, which negating the element
        // schema is the only way to write before Draft 6. Draft 4 reads `1` and `1.0` as one member
        // and `type: integer` takes only the first, so the demand splits the member.
        105 => json!({ "allOf": [
            { "enum": [[small_int(tc)]] },
            { "not": { "items": { "not": { "type": "integer" } } } }
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

fn draw_reference_uri(tc: &TestCase) -> &'static str {
    tc.draw(gs::sampled_from(vec![
        // The document root binds to the document a node was read against rather than to a key of
        // the map, so it is the one name two operands cannot merge by renaming. Without it here no
        // generated schema reads `#` while also carrying `$defs`.
        "#",
        "#/$defs/null_target",
        "#/$defs/integer_target",
        "#/$defs/string_target",
        "#/$defs/raw_target",
        "#/$defs/alias_target",
        "#/$defs/recursive_target",
        "#/$defs/object_target",
        "#/$defs/array_target",
    ]))
}

fn draw_reference_leaf(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(16)) {
        0 => json!({ "$ref": draw_reference_uri(tc) }),
        1 => json!({ "not": { "$ref": draw_reference_uri(tc) } }),
        2 => json!({
            "allOf": [
                { "$ref": "#/$defs/integer_target" },
                { "$ref": "#/$defs/string_target" }
            ]
        }),
        3 => json!({
            "oneOf": [
                { "$ref": "#/$defs/integer_target" },
                { "$ref": "#/$defs/string_target" }
            ]
        }),
        4 => json!({
            "if": { "$ref": "#/$defs/integer_target" },
            "then": { "$ref": "#/$defs/string_target" }
        }),
        5 => json!({ "type": "array", "items": { "$ref": draw_reference_uri(tc) } }),
        6 => json!({ "type": "array", "contains": { "$ref": draw_reference_uri(tc) } }),
        7 => json!({ "type": "object", "properties": { "a": { "$ref": draw_reference_uri(tc) } } }),
        8 => {
            json!({ "type": "object", "additionalProperties": { "$ref": draw_reference_uri(tc) } })
        }
        9 => {
            json!({ "type": "object", "patternProperties": { "^a": { "$ref": draw_reference_uri(tc) } } })
        }
        10 => json!({ "type": "object", "propertyNames": { "$ref": "#/$defs/string_target" } }),
        11 => json!({ "dependentSchemas": { "a": { "$ref": draw_reference_uri(tc) } } }),
        12 => json!({ "not": { "$ref": "#/$defs/recursive_target" } }),
        // A $ref sibling, and $ref inside allOf/oneOf, beside unevaluatedProperties.
        13 => json!({ "$ref": "#/$defs/object_target", "unevaluatedProperties": false }),
        14 => json!({
            "allOf": [{ "$ref": "#/$defs/object_target" }],
            "unevaluatedProperties": false
        }),
        15 => json!({
            "oneOf": [{ "$ref": "#/$defs/object_target" }],
            "unevaluatedProperties": false
        }),
        16 => json!({ "$ref": "#/$defs/array_target", "unevaluatedItems": false }),
        _ => json!({ "$ref": draw_reference_uri(tc), "type": draw_type(tc) }),
    }
}

// An acyclic $ref beside unevaluated*: no-op `true`, bare sibling, allOf/oneOf, and an identical-
// target diamond (not a cycle).
fn draw_ref_unevaluated_leaf(tc: &TestCase) -> Value {
    let target = tc.draw(gs::sampled_from(vec![
        "#/$defs/object_target",
        "#/$defs/array_target",
    ]));
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(5)) {
        0 => json!({ "unevaluatedProperties": true, "$ref": target }),
        1 => json!({ "unevaluatedItems": true, "$ref": target }),
        2 => json!({ "$ref": target, "unevaluatedProperties": false }),
        3 => json!({ "allOf": [{ "$ref": target }], "unevaluatedProperties": false }),
        4 => json!({ "oneOf": [{ "$ref": target }], "unevaluatedProperties": false }),
        _ => json!({
            "oneOf": [{ "$ref": target }, { "$ref": target }],
            "unevaluatedProperties": false
        }),
    }
}

fn draw_schema_node(tc: &TestCase, depth: u32) -> Value {
    if depth == 0 || tc.draw(gs::booleans()) {
        return if tc.draw(gs::integers::<u8>().min_value(0).max_value(7)) == 0 {
            draw_reference_leaf(tc)
        } else {
            draw_leaf(tc)
        };
    }
    let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(2));
    let branches: Vec<Value> = (0..count)
        .map(|_| draw_schema_node(tc, depth - 1))
        .collect();
    if tc.draw(gs::booleans()) {
        json!({ "allOf": branches })
    } else {
        json!({ "anyOf": branches })
    }
}

/// The `$defs` pool every generated schema in this module can reference.
fn shared_defs() -> Value {
    json!({
        "null_target": { "type": "null" },
        "integer_target": { "type": "integer", "minimum": -2 },
        "string_target": { "type": "string", "minLength": 1 },
        "raw_target": { "anyOf": [{}], "unevaluatedProperties": false },
        "alias_target": { "$ref": "#/$defs/integer_target" },
        "recursive_target": {
            "type": "object",
            "properties": { "a": { "$ref": "#/$defs/recursive_target" } }
        },
        "object_target": { "type": "object", "properties": { "z": { "type": "integer" } } },
        "array_target": { "type": "array", "prefixItems": [{ "type": "integer" }] }
    })
}

fn draw_schema(tc: &TestCase, depth: u32) -> Value {
    let mut schema = draw_schema_node(tc, depth);
    if let Value::Object(object) = &mut schema {
        object.insert("$defs".into(), shared_defs());
    }
    schema
}

/// Whether `schema` names the document root anywhere inside it.
///
/// A body that reads `#` cannot be moved out of the root position: wrapped in anything, `#` names
/// the wrapper rather than the body, so the rewrite no longer accepts the values it started with.
fn names_document_root(schema: &Value) -> bool {
    match schema {
        Value::Object(map) => {
            map.get("$ref").and_then(Value::as_str) == Some("#")
                || map.values().any(names_document_root)
        }
        Value::Array(items) => items.iter().any(names_document_root),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn split_root_definitions(schema: &Value) -> (Value, Option<Value>) {
    let mut schema = schema.clone();
    let definitions = schema
        .as_object_mut()
        .and_then(|object| object.remove("$defs"));
    (schema, definitions)
}

fn attach_root_definitions(mut schema: Value, definitions: Option<Value>) -> Value {
    if let Some(definitions) = definitions {
        schema
            .as_object_mut()
            .expect("a transformed schema with root definitions is an object")
            .insert("$defs".into(), definitions);
    }
    schema
}

// Meta-valid keywords the canonicaliser does not model; a document carrying one stays `Raw`.
fn draw_unsupported_leaf(tc: &TestCase) -> Value {
    match tc.draw(gs::integers::<u8>().min_value(0).max_value(4)) {
        0 => json!({ "anyOf": [{}], "unevaluatedProperties": { "type": "integer" } }),
        1 => json!({ "not": { "pattern": "^a" } }),
        2 => json!({ "contains": {}, "unevaluatedItems": { "type": "null" } }),
        3 => json!({ "format": "email" }),
        _ => json!({ "oneOf": [{ "type": "string" }, { "minLength": 1 }] }),
    }
}

fn draw_broad_schema(tc: &TestCase, depth: u32) -> Value {
    if depth == 0 || tc.draw(gs::booleans()) {
        return if tc.draw(gs::booleans()) {
            draw_leaf(tc)
        } else {
            draw_unsupported_leaf(tc)
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

// An acyclic $ref beside unevaluatedProperties/unevaluatedItems must canonicalize to a modeled form.
#[hegel::test(test_cases = 5_000)]
fn ref_unevaluated_never_stays_raw(tc: TestCase) {
    let draft = draw_draft(&tc);
    let schema = attach_root_definitions(draw_ref_unevaluated_leaf(&tc), Some(shared_defs()));
    let Ok(canonical) = jsonschema::canonical::options()
        .with_draft(draft)
        .canonicalize(&schema)
    else {
        return;
    };
    assert_ne!(
        canonical.kind(),
        jsonschema::canonical::CanonicalKind::Raw,
        "schema = {schema}"
    );
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let emitted = canonical.to_json_schema();
    let (Ok(raw), Ok(modeled)) = (build(&schema), build(&emitted)) else {
        return;
    };
    let instance = tc.draw(arbitrary_instance());
    assert_eq!(
        raw.is_valid(&instance),
        modeled.is_valid(&instance),
        "{schema} vs {emitted} on {instance}"
    );
}

// A cyclic $ref beside unevaluatedProperties must still bail to Raw. Cycle runs through allOf/$ref,
// the only things property_cover recurses into. Draft fixed at 2020-12: earlier drafts don't know
// `unevaluatedProperties`, so this would reduce to a bare reference ring that canonicalizes to `True`.
#[hegel::test(test_cases = 2_000)]
fn cyclic_ref_unevaluated_stays_raw(_tc: TestCase) {
    let draft = Draft::Draft202012;
    let schema = json!({
        "$defs": { "loop": { "allOf": [{ "$ref": "#/$defs/loop" }] } },
        "$ref": "#/$defs/loop",
        "unevaluatedProperties": false
    });
    let Ok(canonical) = jsonschema::canonical::options()
        .with_draft(draft)
        .canonicalize(&schema)
    else {
        return;
    };
    assert_eq!(
        canonical.kind(),
        jsonschema::canonical::CanonicalKind::Raw,
        "schema = {schema}"
    );
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
        assert_eq!(once, twice, "schema = {schema}");
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
    // A drawn instance almost never lands on a value the schema names, and those are where a value
    // set that lost or gained a member shows.
    let mut named = Vec::new();
    named_values(&schema, &mut named);
    for instance in named.iter().chain(std::iter::once(&instance)) {
        assert_eq!(
            raw.is_valid(instance),
            canonical.is_valid(instance),
            "{schema} vs {emitted} on {instance}"
        );
    }
}

// Every value the schema names through `const` or `enum`.
fn named_values(schema: &Value, into: &mut Vec<Value>) {
    match schema {
        Value::Object(map) => {
            if let Some(value) = map.get("const") {
                into.push(value.clone());
            }
            if let Some(Value::Array(values)) = map.get("enum") {
                into.extend(values.iter().cloned());
            }
            for value in map.values() {
                named_values(value, into);
            }
        }
        Value::Array(items) => {
            for item in items {
                named_values(item, into);
            }
        }
        _ => {}
    }
}

// The complement of a complement accepts what the schema accepts. Taking it through the algebra
// runs both steps: spelling the pair as nested `not` keywords cancels them before either one runs,
// leaving nothing to check.
#[hegel::test(test_cases = 5_000)]
fn negating_a_complement_restores_the_accepted_values(tc: TestCase) {
    let draft = draw_draft(&tc);
    let validate_formats = tc.draw(gs::booleans());
    let schema = draw_schema(&tc, 2);
    let instance = tc.draw(arbitrary_instance());
    let Ok(canonical) = jsonschema::canonical::options()
        .with_draft(draft)
        .should_validate_formats(validate_formats)
        .canonicalize(&schema)
    else {
        return;
    };
    let Ok(complement) = canonical.negate() else {
        return;
    };
    let Ok(restored) = complement.negate() else {
        return;
    };
    let build = |value: &Value| {
        jsonschema::options()
            .with_draft(draft)
            .should_validate_formats(validate_formats)
            .build(value)
    };
    let emitted = restored.to_json_schema();
    let (Ok(raw), Ok(twice)) = (build(&schema), build(&emitted)) else {
        return;
    };
    assert_eq!(
        raw.is_valid(&instance),
        twice.is_valid(&instance),
        "schema = {schema}\n  restored = {emitted}\n  instance = {instance}"
    );
}

// Every set operation answers about the values its operands accept, so a validator built from the
// result agrees with the two it was combined from - on the union, the difference, and the coverage
// each of them decides.
#[hegel::test(test_cases = 5_000)]
fn set_operations_answer_about_the_values_their_operands_accept(tc: TestCase) {
    let draft = draw_draft(&tc);
    let validate_formats = tc.draw(gs::booleans());
    let left_source = draw_schema(&tc, 2);
    let right_source = draw_schema(&tc, 2);
    let instance = tc.draw(arbitrary_instance());
    let canonicalize = |value: &Value| {
        jsonschema::canonical::options()
            .with_draft(draft)
            .should_validate_formats(validate_formats)
            .canonicalize(value)
    };
    let build = |value: &Value| {
        jsonschema::options()
            .with_draft(draft)
            .should_validate_formats(validate_formats)
            .build(value)
    };
    let (Ok(left), Ok(right)) = (canonicalize(&left_source), canonicalize(&right_source)) else {
        return;
    };
    let (Ok(left_validator), Ok(right_validator)) = (build(&left_source), build(&right_source))
    else {
        return;
    };
    let in_left = left_validator.is_valid(&instance);
    let in_right = right_validator.is_valid(&instance);
    for (name, result, expected) in [
        ("union", left.union(&right), in_left || in_right),
        ("intersection", left.intersect(&right), in_left && in_right),
        ("difference", left.subtract(&right), in_left && !in_right),
    ] {
        let Ok(result) = result else {
            continue;
        };
        let emitted = result.to_json_schema();
        let context =
            || format!("\n  left = {left_source}\n  right = {right_source}\n  {name} = {emitted}");
        // Reading a result back must settle: set operations resolve references where parsing keeps
        // them symbolic, so one more round can fold further, but no round after that may.
        if let Ok(once) = canonicalize(&emitted) {
            let once = once.to_json_schema();
            if let Ok(twice) = canonicalize(&once) {
                assert_eq!(
                    twice.to_json_schema(),
                    once,
                    "reading the {name} back does not settle{}",
                    context()
                );
            }
            if let Ok(validator) = build(&once) {
                assert_eq!(
                    expected,
                    validator.is_valid(&instance),
                    "the {name} read back disagrees{}\n  read back = {once}\n  instance = {instance}",
                    context()
                );
            }
        }
        // The engine may decline to answer, but never gets the answer wrong.
        if result.satisfiability() == Satisfiability::No {
            assert!(
                !expected,
                "the {name} folded to nothing over an instance it accepts{}\n  instance = {instance}",
                context()
            );
        }
        if let Ok(validator) = build(&emitted) {
            assert_eq!(
                expected,
                validator.is_valid(&instance),
                "the {name} disagrees{}\n  instance = {instance}",
                context()
            );
        }
    }
    // One value set has one form, whichever way round the commutative operations are asked.
    for (name, forward, backward) in [
        ("union", left.union(&right), right.union(&left)),
        (
            "intersection",
            left.intersect(&right),
            right.intersect(&left),
        ),
    ] {
        if let (Ok(forward), Ok(backward)) = (forward, backward) {
            assert_eq!(
                forward.to_json_schema(),
                backward.to_json_schema(),
                "{name} is not commutative\n  left = {left_source}\n  right = {right_source}"
            );
        }
    }
    assert_set_algebra_laws(&[(&left, &left_source), (&right, &right_source)], draft);
    // A coverage the engine decides must hold on every instance: `Yes` leaves no value of the
    // argument outside the receiver, `No` leaves at least one - which the difference exhibits.
    match left.covers(&right) {
        Ok(Containment::Yes) => assert!(
            !in_right || in_left,
            "covers said yes\n  left = {left_source}\n  right = {right_source}\n  instance = {instance}"
        ),
        Ok(Containment::No) => {
            let Ok(difference) = right.subtract(&left) else {
                return;
            };
            assert_ne!(
                difference.satisfiability(),
                Satisfiability::No,
                "covers said no over an empty difference\n  left = {left_source}\n  right = {right_source}"
            );
        }
        Ok(Containment::Unknown) | Err(_) => {}
    }
}

/// The laws every operand obeys with itself and with the two constants, checked on the form each
/// operation hands back: these are exact whatever the operands are spelled like, so a spelling the
/// engine reads through must not change the answer.
fn assert_set_algebra_laws(operands: &[(&CanonicalSchema, &Value)], draft: Draft) {
    let constant = |value: Value| {
        jsonschema::canonical::options()
            .with_draft(draft)
            .canonicalize(&value)
            .expect("a boolean schema canonicalizes")
    };
    let nothing = constant(json!(false));
    let everything = constant(json!(true));
    for (side, source) in operands {
        for (law, result, expected) in [
            ("a | a", side.union(side), *side),
            ("a & a", side.intersect(side), *side),
            ("a | nothing", side.union(&nothing), *side),
            ("a & everything", side.intersect(&everything), *side),
            ("a | everything", side.union(&everything), &everything),
            ("a & nothing", side.intersect(&nothing), &nothing),
            ("a \\ a", side.subtract(side), &nothing),
            ("a \\ nothing", side.subtract(&nothing), *side),
            ("nothing \\ a", nothing.subtract(side), &nothing),
        ] {
            let Ok(result) = result else {
                continue;
            };
            assert_eq!(
                result.to_json_schema(),
                expected.to_json_schema(),
                "`{law}` does not hold\n  a = {source}"
            );
        }
        // A schema covers itself, whatever it is written like. A `Raw` operand is refused before
        // any of that, which says nothing about coverage.
        if let Ok(containment) = side.covers(side) {
            assert_eq!(
                containment,
                Containment::Yes,
                "a schema does not cover itself\n  a = {source}"
            );
        }
        // The same laws against the same values spelled as a pointer. Asked against the node
        // itself, the wrapper's identity shortcuts answer before the algebra runs.
        // From what the operand emits: it can be a handle on one target of the document `source`
        // spells, and the twin has to hold the operand's values.
        let Some(twin) = pointer_twin(&side.to_json_schema()) else {
            continue;
        };
        let Ok(twin) = jsonschema::canonical::options()
            .with_draft(draft)
            .canonicalize(&twin)
        else {
            continue;
        };
        // The difference need not fold to `false` - proving an `allOf` empty is more than the form
        // promises - but it must accept nothing.
        for (law, difference) in [
            ("a \\ &a", side.subtract(&twin)),
            ("&a \\ a", twin.subtract(side)),
        ] {
            let Ok(difference) = difference else {
                continue;
            };
            assert_ne!(
                difference.satisfiability(),
                Satisfiability::Yes,
                "`{law}` claims a value\n  a = {source}\n  difference = {}",
                difference.to_json_schema()
            );
            let Ok(validator) = jsonschema::options()
                .with_draft(draft)
                .build(&difference.to_json_schema())
            else {
                continue;
            };
            for instance in instance_pool() {
                assert!(
                    !validator.is_valid(&instance),
                    "`{law}` accepts {instance}\n  a = {source}\n  difference = {}",
                    difference.to_json_schema()
                );
            }
        }
        // Coverage may go undecided through a pointer, but never the wrong way round.
        for (law, containment) in [
            ("a covers &a", side.covers(&twin)),
            ("&a covers a", twin.covers(side)),
        ] {
            if let Ok(containment) = containment {
                assert_ne!(
                    containment,
                    Containment::No,
                    "`{law}` was refused\n  a = {source}"
                );
            }
        }
    }
}

/// The same document reached through a pointer: the same values under a form the algebra must read
/// through rather than recognise. `None` where the body names the root, which the wrapper rebinds.
fn pointer_twin(source: &Value) -> Option<Value> {
    const TWIN: &str = "canonical_twin";
    let object = source.as_object()?;
    if names_document_root(source) {
        return None;
    }
    let mut definitions = object
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut body = object.clone();
    body.remove("$defs");
    definitions.insert(TWIN.to_string(), Value::Object(body));
    Some(json!({"$defs": definitions, "$ref": format!("#/$defs/{TWIN}")}))
}

/// The shared pool with one entry replaced, which is what editing a schema looks like.
fn edited_defs(tc: &TestCase, replacement: Value) -> Value {
    let key = tc.draw(gs::sampled_from(vec![
        "null_target",
        "integer_target",
        "string_target",
        "object_target",
        "array_target",
    ]));
    let mut defs = shared_defs();
    defs.as_object_mut()
        .expect("the pool is an object")
        .insert(key.to_string(), replacement);
    defs
}

/// A handle on the document, or on one of the targets it carries: an operand reached through
/// `definition` spells a pointer of its own and reads the rest of its document through it, which is
/// what makes two versions of one document tell each other apart.
fn draw_operand(
    tc: &TestCase,
    schema: &jsonschema::canonical::CanonicalSchema,
) -> jsonschema::canonical::CanonicalSchema {
    let targets: Vec<String> = schema.definitions().map(|(uri, _)| uri).collect();
    if targets.is_empty() || tc.draw(gs::booleans()) {
        return schema.clone();
    }
    let uri = tc.draw(gs::sampled_from(targets));
    schema.definition(&uri).expect("a listed target resolves")
}

// Two versions of one document resolve their pointers through one map, so an entry they disagree on
// leaves them incomparable - and every operation must say so rather than answer about whichever
// body it happened to keep. Whatever an operation does answer holds on every instance.
#[hegel::test(test_cases = 5_000)]
fn set_operations_over_two_versions_of_one_document_answer_or_decline(tc: TestCase) {
    let draft = draw_draft(&tc);
    // A boolean schema carries no `$defs`, so the body is wrapped where it is not an object.
    let body = match draw_schema_node(&tc, 2) {
        object @ Value::Object(_) => object,
        other => json!({ "allOf": [other] }),
    };
    // Half the documents read `#` as well, so the pair exercises the one name a merge cannot rename
    // apart: two versions binding the root to different bodies must decline rather than answer
    // about whichever root the operation happened to keep.
    let body = if tc.draw(gs::booleans()) {
        json!({"type": "object", "allOf": [body], "properties": {"self": {"$ref": "#"}}})
    } else {
        body
    };
    let document = |defs: Value| {
        let mut schema = body.clone();
        schema
            .as_object_mut()
            .expect("the body was wrapped into an object")
            .insert("$defs".into(), defs);
        schema
    };
    let old_source = document(shared_defs());
    let new_source = document(edited_defs(&tc, draw_leaf(&tc)));
    let canonicalize = |value: &Value| {
        jsonschema::canonical::options()
            .with_draft(draft)
            .canonicalize(value)
    };
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let (Ok(old), Ok(new)) = (canonicalize(&old_source), canonicalize(&new_source)) else {
        return;
    };
    let (left, right) = (draw_operand(&tc, &old), draw_operand(&tc, &new));
    // A handle on a target answers about that target, so the validators are built from what it
    // emits rather than from the document it came out of.
    let (Ok(left_validator), Ok(right_validator)) = (
        build(&left.to_json_schema()),
        build(&right.to_json_schema()),
    ) else {
        return;
    };
    let instance = tc.draw(arbitrary_instance());
    let in_left = left_validator.is_valid(&instance);
    let in_right = right_validator.is_valid(&instance);

    for (name, result, expected) in [
        ("union", left.union(&right), in_left || in_right),
        ("intersection", left.intersect(&right), in_left && in_right),
        ("difference", left.subtract(&right), in_left && !in_right),
    ] {
        let Ok(result) = result else {
            continue;
        };
        let emitted = result.to_json_schema();
        let Ok(validator) = build(&emitted) else {
            continue;
        };
        assert_eq!(
            expected,
            validator.is_valid(&instance),
            "{name} disagrees\n  old = {old_source}\n  new = {new_source}\n  left = {}\n  right = {}\n  result = {emitted}\n  instance = {instance}",
            left.to_json_schema(),
            right.to_json_schema()
        );
    }
    // One value set has one answer, whichever way round the commutative operations are asked.
    for (name, forward, backward) in [
        ("union", left.union(&right), right.union(&left)),
        (
            "intersection",
            left.intersect(&right),
            right.intersect(&left),
        ),
    ] {
        if let (Ok(forward), Ok(backward)) = (forward, backward) {
            assert_eq!(
                forward.to_json_schema(),
                backward.to_json_schema(),
                "{name} is not commutative\n  old = {old_source}\n  new = {new_source}\n  left = {}\n  right = {}",
                left.to_json_schema(),
                right.to_json_schema()
            );
        }
    }
    assert_set_algebra_laws(&[(&left, &old_source), (&right, &new_source)], draft);
    if let Ok(Containment::Yes) = left.covers(&right) {
        assert!(
            !in_right || in_left,
            "covers said yes\n  old = {old_source}\n  new = {new_source}\n  instance = {instance}"
        );
    }
}

// A pool in normal form keeps no leaf a sibling of its own kind already holds: an intersection that
// leaves the leaf untouched proves every value it admits lies in the sibling, so the fold that
// drops it was left undone. Leaves of different kinds sit in different pools, which the union
// assembles side by side without weighing one against the other.
#[hegel::test(test_cases = 5_000)]
fn a_union_keeps_no_leaf_a_sibling_of_its_kind_already_holds(tc: TestCase) {
    let draft = draw_draft(&tc);
    let validate_formats = tc.draw(gs::booleans());
    let seed = draw_schema(&tc, 2);
    if names_document_root(&seed) {
        return;
    }
    let (body, definitions) = split_root_definitions(&seed);
    // A branch beside a narrowing of itself: the narrowing adds nothing, so the union must shed it.
    let mut branches = vec![body.clone(), json!({ "allOf": [body, draw_leaf(&tc)] })];
    let count = tc.draw(gs::integers::<usize>().min_value(0).max_value(2));
    for _ in 0..count {
        branches.push(draw_schema_node(&tc, 2));
    }
    let schema = attach_root_definitions(json!({ "anyOf": branches }), definitions);
    let Ok(canonical) = jsonschema::canonical::options()
        .with_draft(draft)
        .should_validate_formats(validate_formats)
        .canonicalize(&schema)
    else {
        return;
    };
    let mut nodes = Vec::new();
    collect_nodes(&canonical, &mut nodes);
    for node in &nodes {
        let CanonicalView::AnyOf(branches) = node.view() else {
            continue;
        };
        for branch in &branches {
            let Some(pool) = leaf_pool(branch) else {
                continue;
            };
            for sibling in &branches {
                if leaf_pool(sibling) != Some(pool) || sibling == branch {
                    continue;
                }
                // One divisor standing for another is decided by the arithmetic their spellings
                // share, which is a wider question than the facets a pool weighs.
                if divisors(branch) != divisors(sibling) {
                    continue;
                }
                // Minimization drops what a sibling absorbs, which is what intersecting the two
                // back into the branch says. Coverage the difference decides - the empty string
                // failing a media type, say - is a wider question than the facets a pool weighs.
                assert!(
                    !matches!(sibling.intersect(branch), Ok(shared) if &shared == branch),
                    "schema = {schema}\n  branch = {}\n  sibling = {}",
                    branch.to_json_schema(),
                    sibling.to_json_schema()
                );
            }
        }
    }
}

/// The divisors a numeric leaf carries, required and barred.
fn divisors(schema: &CanonicalSchema) -> (Vec<serde_json::Number>, Vec<serde_json::Number>) {
    match schema.view() {
        CanonicalView::Integer(leaf) => (leaf.multiple_of, leaf.not_multiple_of),
        CanonicalView::Number(leaf) => (leaf.multiple_of, leaf.not_multiple_of),
        _ => (Vec::new(), Vec::new()),
    }
}

/// The pool a union collects this branch into, or `None` for a leaf whose pool weighs nested
/// schemas rather than facets alone - the algebra compares a nested schema only against itself.
fn leaf_pool(schema: &CanonicalSchema) -> Option<JsonType> {
    match schema.view() {
        CanonicalView::String(_) => Some(JsonType::String),
        CanonicalView::Integer(_) => Some(JsonType::Integer),
        CanonicalView::Number(_) => Some(JsonType::Number),
        _ => None,
    }
}

// Every node of a canonical form, the root included.
fn collect_nodes(schema: &CanonicalSchema, into: &mut Vec<CanonicalSchema>) {
    into.push(schema.clone());
    match schema.view() {
        CanonicalView::TypedGroup(group) => collect_nodes(&group.body, into),
        CanonicalView::Not(operand) => collect_nodes(&operand, into),
        CanonicalView::AllOf(branches)
        | CanonicalView::AnyOf(branches)
        | CanonicalView::OneOf(branches) => {
            for branch in &branches {
                collect_nodes(branch, into);
            }
        }
        CanonicalView::Array(array) => {
            for item in &array.prefix_items {
                collect_nodes(item, into);
            }
            if let Some(items) = &array.items {
                collect_nodes(items, into);
            }
            for facet in &array.contains {
                collect_nodes(&facet.schema, into);
            }
        }
        CanonicalView::Object(object) => {
            if let Some(names) = &object.property_names {
                collect_nodes(names, into);
            }
            for entry in object.properties.values() {
                collect_nodes(entry, into);
            }
            for entry in object.pattern_properties.values() {
                collect_nodes(entry, into);
            }
            if let Some(additional) = &object.additional_properties {
                collect_nodes(additional, into);
            }
        }
        CanonicalView::MultiType(_)
        | CanonicalView::String(_)
        | CanonicalView::Integer(_)
        | CanonicalView::Number(_)
        | CanonicalView::Const(_)
        | CanonicalView::Enum(_)
        | CanonicalView::Reference(_)
        | CanonicalView::True
        | CanonicalView::False
        | CanonicalView::Raw(_) => {}
    }
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
// An unsupported document is kept as written, so only modeled ones carry the claim.
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
    let (schema, definitions) = split_root_definitions(schema);
    let rewritten = match tc.draw(gs::integers::<u8>().min_value(0).max_value(5)) {
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
        5 => split_keywords(&schema).unwrap_or_else(|| json!({ "allOf": [schema] })),
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
            _ => split_keywords(&schema).unwrap_or_else(|| json!({ "allOf": [schema] })),
        },
    };
    attach_root_definitions(rewritten, definitions)
}

// Keywords that constrain on their own, so moving one into its own `allOf` branch accepts the same
// values. A pair that works together - `contentEncoding` with `contentMediaType`, `properties` with
// `additionalProperties` - does not belong here.
const SEPARABLE_KEYWORDS: [&str; 5] = ["type", "minLength", "maxLength", "pattern", "format"];

/// The same keywords, one per `allOf` branch, or `None` where the object holds one that reads its
/// neighbours.
fn split_keywords(schema: &Value) -> Option<Value> {
    let object = schema.as_object()?;
    if object.len() < 2
        || !object
            .keys()
            .all(|key| SEPARABLE_KEYWORDS.contains(&key.as_str()))
    {
        return None;
    }
    Some(json!({
        "allOf": object
            .iter()
            .map(|(key, value)| json!({ key.clone(): value }))
            .collect::<Vec<_>>()
    }))
}

#[hegel::test(test_cases = 5_000)]
fn equality_preserving_rewrites_converge(tc: TestCase) {
    let draft = draw_draft(&tc);
    let schema = draw_schema(&tc, 2);
    // Draft 4's metaschema rejects boolean subschemas, so a wrap of a boolean root is not a
    // meta-valid document there.
    if !schema.is_object() || names_document_root(&schema) {
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
    if names_document_root(&schema) {
        return;
    }
    let (schema_body, definitions) = split_root_definitions(&schema);
    let negated = attach_root_definitions(json!({ "not": schema_body }), definitions);
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

#[derive(Clone, Copy, Debug)]
enum Link {
    /// Consumes structure: the reference sits under a required property.
    RequiredProperty,
    /// Consumes structure: the reference sits under `items` beside a non-zero `minItems`.
    Item,
    /// Consumes structure: the reference sits under an optional property.
    OptionalProperty,
    /// Consumes structure: the reference sits under `contains`.
    Contains,
    /// Consumes structure: the reference sits under `propertyNames` beside `minProperties`.
    PropertyName,
    /// Consumes nothing: the reference sits beside a type constraint in an `allOf`.
    InPlace,
    /// Consumes nothing, and one branch terminates: an `anyOf` needs both branches to fail.
    InPlaceBranch,
    /// Two consuming edges from one body.
    TwoRequired,
    /// Two consuming edges, both demanded by `minItems`.
    PrefixPair,
    /// One in-place edge and one consuming edge from the same body.
    MixedPosition,
    /// Two in-place edges under `oneOf`.
    OneOfPair,
    /// An in-place branch beside a consuming one, under `anyOf`.
    AnyOfMixed,
    /// A consuming edge conjoined with an in-place one.
    ContainsMixed,
    /// Terminates the ring.
    Base,
    /// A bare reference, carrying no assertion at all.
    Bare,
    /// Two bare references conjoined, carrying no assertion at all.
    BareConjunction,
    /// Two bare references in a branch, carrying no assertion at all.
    BareBranch,
}

/// Links that always consume structure, so every cycle they build is well founded.
const WELL_FOUNDED_LINKS: &[Link] = &[
    Link::RequiredProperty,
    Link::Item,
    Link::OptionalProperty,
    Link::Contains,
    Link::PropertyName,
    Link::TwoRequired,
    Link::PrefixPair,
    Link::Base,
];

/// Links that consume a *demanded* piece of the instance: satisfying one needs a strictly smaller
/// value satisfying its target. A ring built from these alone descends forever.
const DEMANDING_LINKS: &[Link] = &[
    Link::RequiredProperty,
    Link::Item,
    Link::Contains,
    Link::PropertyName,
    Link::TwoRequired,
    Link::PrefixPair,
];

/// Links carrying no assertion whatsoever, so a ring built from these alone constrains nothing.
const BARE_LINKS: &[Link] = &[Link::Bare, Link::BareConjunction, Link::BareBranch];

const ALL_LINKS: &[Link] = &[
    Link::RequiredProperty,
    Link::Item,
    Link::OptionalProperty,
    Link::Contains,
    Link::PropertyName,
    Link::InPlace,
    Link::InPlaceBranch,
    Link::TwoRequired,
    Link::PrefixPair,
    Link::MixedPosition,
    Link::OneOfPair,
    Link::AnyOfMixed,
    Link::ContainsMixed,
    Link::Base,
    Link::Bare,
    Link::BareConjunction,
    Link::BareBranch,
];

/// A definition body reaching `next`, or `second` too when the arm takes two targets.
fn definition_body(link: Link, next: &str, second: &str) -> Value {
    match link {
        Link::RequiredProperty => {
            json!({"type": "object", "required": ["x"], "properties": {"x": {"$ref": next}}})
        }
        Link::Item => json!({"type": "array", "minItems": 1, "items": {"$ref": next}}),
        Link::OptionalProperty => json!({"type": "object", "properties": {"x": {"$ref": next}}}),
        Link::Contains => json!({"type": "array", "contains": {"$ref": next}}),
        Link::PropertyName => {
            json!({"type": "object", "minProperties": 1, "propertyNames": {"$ref": next}})
        }
        Link::Bare => json!({"$ref": next}),
        Link::BareConjunction => json!({"allOf": [{"$ref": next}, {"$ref": second}]}),
        Link::BareBranch => json!({"anyOf": [{"$ref": next}, {"$ref": second}]}),
        Link::InPlace => json!({"allOf": [{"$ref": next}, {"type": "integer"}]}),
        Link::InPlaceBranch => json!({"anyOf": [{"$ref": next}, {"type": "integer"}]}),
        // Two outgoing references, which one target per body can never produce - and which every
        // shape the guardedness rule decides needs: a member on both an in-place and a consuming
        // edge, or a cycle that only closes through a definition outside it.
        Link::TwoRequired => json!({"type": "object", "required": ["x", "y"], "properties": {
            "x": {"$ref": next}, "y": {"$ref": second}
        }}),
        Link::PrefixPair => json!({"type": "array", "minItems": 2, "prefixItems": [
            {"$ref": next}, {"$ref": second}
        ]}),
        Link::MixedPosition => json!({"allOf": [
            {"$ref": next},
            {"type": "object", "required": ["x"], "properties": {"x": {"$ref": second}}}
        ]}),
        Link::AnyOfMixed => json!({"anyOf": [
            {"$ref": next},
            {"type": "object", "required": ["x"], "properties": {"x": {"$ref": second}}}
        ]}),
        Link::ContainsMixed => json!({"allOf": [
            {"type": "array", "contains": {"$ref": next}},
            {"$ref": second}
        ]}),
        Link::OneOfPair => json!({"oneOf": [{"$ref": next}, {"$ref": second}]}),
        Link::Base => json!({"type": "integer"}),
    }
}

/// A graph of definitions, each reaching one or two others by index.
///
/// Every target lands on the root or a definition, so the whole document is closed under the
/// reference edges and `links` alone decides what the ring carries. [`WELL_FOUNDED_LINKS`] is the
/// set the validator can serve as an oracle for: its own recursion guard is spelling-sensitive on
/// an ill-founded cycle - reordering two `oneOf` branches flips its verdict with no
/// canonicalization involved.
#[hegel::composite]
fn definition_graph(tc: &TestCase, links: &'static [Link]) -> Value {
    let size = tc.draw(gs::integers::<usize>().min_value(1).max_value(5));
    let target = |tc: &TestCase| {
        let index = tc.draw(gs::integers::<usize>().min_value(0).max_value(size));
        if index == size {
            "#".to_string()
        } else {
            format!("#/$defs/d{index}")
        }
    };
    let body = |tc: &TestCase| {
        let link = tc.draw(gs::sampled_from(links.to_vec()));
        definition_body(link, &target(tc), &target(tc))
    };
    let mut definitions = serde_json::Map::new();
    for index in 0..size {
        definitions.insert(format!("d{index}"), body(tc));
    }
    let mut root = body(tc)
        .as_object()
        .expect("a link body is an object")
        .clone();
    root.insert("$defs".into(), Value::Object(definitions));
    Value::Object(root)
}

fn emptiness_candidates() -> Vec<Value> {
    vec![
        json!(null),
        json!(true),
        json!(1),
        json!("x"),
        json!([]),
        json!([1]),
        json!([[1]]),
        json!([{}]),
        json!({}),
        json!({"x": {}}),
        json!({"x": 1}),
        json!({"x": {"x": 1}}),
        json!({"x": []}),
    ]
}

// Canonicalizing a recursive definition ring preserves the accepted set.
#[hegel::test(test_cases = 5_000)]
fn recursive_reference_form_preserves_validation(tc: TestCase) {
    let schema = tc.draw(definition_graph(WELL_FOUNDED_LINKS));
    let emitted = canonicalize(&schema, Draft::Draft202012)
        .unwrap_or_else(|| panic!("a definition ring canonicalizes: {schema}"));
    let build = |value: &Value| {
        jsonschema::options()
            .with_draft(Draft::Draft202012)
            .build(value)
            .unwrap_or_else(|error| panic!("a definition ring builds: {error}\n  {value}"))
    };
    let (raw, canonical) = (build(&schema), build(&emitted));
    for instance in emptiness_candidates() {
        assert_eq!(
            raw.is_valid(&instance),
            canonical.is_valid(&instance),
            "{schema} vs {emitted} on {instance}"
        );
    }
}

#[hegel::test(test_cases = 5_000)]
fn unsatisfiable_recursive_reference_rejects_every_candidate(tc: TestCase) {
    let schema = tc.draw(definition_graph(WELL_FOUNDED_LINKS));
    let canonical = jsonschema::canonical::options()
        .with_draft(Draft::Draft202012)
        .canonicalize(&schema)
        .unwrap_or_else(|error| panic!("a definition ring canonicalizes: {error}\n  {schema}"));
    if canonical.satisfiability() != Satisfiability::No {
        return;
    }
    let validator = jsonschema::options()
        .with_draft(Draft::Draft202012)
        .build(&schema)
        .unwrap_or_else(|error| panic!("a definition ring builds: {error}\n  {schema}"));
    for instance in emptiness_candidates() {
        assert!(
            !validator.is_valid(&instance),
            "declared unsatisfiable but the validator accepts {instance}\n  schema = {schema}"
        );
    }
}

// Satisfying a demanding link needs a strictly smaller value satisfying its target, and every
// target lands back inside the ring, so a value satisfying any member would carry an infinite
// descending chain of sub-values. JSON values are finite, so no member admits anything - the
// unsatisfiability is a theorem of the construction rather than a verdict to be read off.
#[hegel::test(test_cases = 5_000)]
fn a_demanding_reference_ring_admits_nothing(tc: TestCase) {
    let schema = tc.draw(definition_graph(DEMANDING_LINKS));
    let canonical = jsonschema::canonical::options()
        .with_draft(Draft::Draft202012)
        .canonicalize(&schema)
        .unwrap_or_else(|error| panic!("a definition ring canonicalizes: {error}\n  {schema}"));
    assert_eq!(
        canonical.satisfiability(),
        Satisfiability::No,
        "a ring of demanded sub-values admits nothing, but the canonical form is {}\n  schema = {schema}",
        canonical.to_json_schema()
    );
}

// A ring of bare references asserts nothing anywhere, so every value walks it forever without ever
// meeting a constraint that could reject it. Its canonical form is `true`.
#[hegel::test(test_cases = 5_000)]
fn a_bare_reference_ring_admits_everything(tc: TestCase) {
    let schema = tc.draw(definition_graph(BARE_LINKS));
    let canonical = jsonschema::canonical::options()
        .with_draft(Draft::Draft202012)
        .canonicalize(&schema)
        .unwrap_or_else(|error| panic!("a definition ring canonicalizes: {error}\n  {schema}"));
    assert_eq!(
        canonical.view(),
        CanonicalView::True,
        "a ring carrying no assertion admits every value\n  schema = {schema}"
    );
}

#[hegel::test(test_cases = 30_000)]
fn recursive_reference_form_is_idempotent(tc: TestCase) {
    let schema = tc.draw(definition_graph(ALL_LINKS));
    let once = canonicalize(&schema, Draft::Draft202012)
        .unwrap_or_else(|| panic!("a definition ring canonicalizes: {schema}"));
    let twice = canonicalize(&once, Draft::Draft202012)
        .unwrap_or_else(|| panic!("a canonical form re-canonicalizes: {once}"));
    assert_eq!(once, twice, "schema = {schema}");
}

// Composed expressions answer about the values their operands accept, whatever the grouping: an
// intermediate result re-enters the algebra as an operand, where a dispatch written per ordered
// pair must not let the association or the spelling decide the answer.
#[hegel::test(test_cases = 5_000)]
fn composed_set_operations_answer_about_the_values_their_operands_accept(tc: TestCase) {
    let draft = draw_draft(&tc);
    let sources = [
        draw_schema(&tc, 2),
        draw_schema(&tc, 2),
        draw_schema(&tc, 2),
    ];
    let instance = tc.draw(arbitrary_instance());
    let canonicalize = |value: &Value| {
        jsonschema::canonical::options()
            .with_draft(draft)
            .canonicalize(value)
    };
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let mut operands = Vec::new();
    let mut admitted = Vec::new();
    for source in &sources {
        let (Ok(operand), Ok(validator)) = (canonicalize(source), build(source)) else {
            return;
        };
        admitted.push(validator.is_valid(&instance));
        operands.push(operand);
    }
    let (a, b, c) = (&operands[0], &operands[1], &operands[2]);
    let (in_a, in_b, in_c) = (admitted[0], admitted[1], admitted[2]);
    let pair = |left: Result<CanonicalSchema, _>,
                op: fn(
        &CanonicalSchema,
        &CanonicalSchema,
    )
        -> Result<CanonicalSchema, jsonschema::canonical::CanonicalizationError>,
                right: &CanonicalSchema| { left.and_then(|left| op(&left, right)) };
    let expressions = [
        (
            "(a & b) & c",
            pair(a.intersect(b), CanonicalSchema::intersect, c),
            in_a && in_b && in_c,
        ),
        (
            "a & (b & c)",
            pair(b.intersect(c), CanonicalSchema::intersect, a),
            in_a && in_b && in_c,
        ),
        (
            "(a | b) | c",
            pair(a.union(b), CanonicalSchema::union, c),
            in_a || in_b || in_c,
        ),
        (
            "a | (b | c)",
            pair(b.union(c), CanonicalSchema::union, a),
            in_a || in_b || in_c,
        ),
        (
            "a & (b | c)",
            pair(b.union(c), CanonicalSchema::intersect, a),
            in_a && (in_b || in_c),
        ),
        (
            "(a | b) \\ c",
            pair(a.union(b), CanonicalSchema::subtract, c),
            (in_a || in_b) && !in_c,
        ),
        (
            "(a \\ b) \\ c",
            pair(a.subtract(b), CanonicalSchema::subtract, c),
            in_a && !in_b && !in_c,
        ),
        (
            "a & !b",
            pair(b.negate(), CanonicalSchema::intersect, a),
            in_a && !in_b,
        ),
        (
            "a | (a & b)",
            pair(a.intersect(b), CanonicalSchema::union, a),
            in_a,
        ),
    ];
    for (law, result, expected) in expressions {
        let Ok(result) = result else {
            continue;
        };
        let emitted = result.to_json_schema();
        let context = || {
            format!(
                "\n  a = {}\n  b = {}\n  c = {}\n  {law} = {emitted}\n  instance = {instance}",
                sources[0], sources[1], sources[2]
            )
        };
        if result.satisfiability() == Satisfiability::No {
            assert!(
                !expected,
                "`{law}` folded to nothing over an instance it accepts{}",
                context()
            );
        }
        let Ok(validator) = build(&emitted) else {
            continue;
        };
        assert_eq!(
            expected,
            validator.is_valid(&instance),
            "`{law}` disagrees{}",
            context()
        );
    }
}

fn instance_pool() -> Vec<Value> {
    vec![
        json!(null),
        json!(true),
        json!(false),
        json!(0),
        json!(1),
        json!(-2),
        json!(1.5),
        json!(2.0),
        json!(""),
        json!("a"),
        json!("ab"),
        json!("b"),
        json!("xb"),
        json!("aab"),
        json!([]),
        json!([1]),
        json!([1, 1]),
        json!([1, "a"]),
        json!([[1]]),
        json!({}),
        json!({"a": 1}),
        json!({"ab": 1}),
        json!({"b": 1}),
        json!({"x": 1}),
        json!({"xb": 1}),
        json!({"a": "a"}),
        json!({"a": {}}),
        json!({"a": {"b": 1}}),
        json!({"a": 1, "b": 1}),
        json!({"a": 1, "ab": 1, "b": 1, "x": 1}),
        json!({"xb": {"a": 1}}),
        json!({"x": {"ab": 1}}),
    ]
}

// The same laws as the drawn-instance property, but read over a fixed pool: one drawn instance
// separates two forms only where it happens to fall between them.
#[hegel::test(test_cases = 5_000)]
fn set_operations_agree_with_their_operands_over_a_pool(tc: TestCase) {
    let draft = draw_draft(&tc);
    let left_source = draw_schema(&tc, 2);
    let right_source = draw_schema(&tc, 2);
    let canonicalize = |value: &Value| {
        jsonschema::canonical::options()
            .with_draft(draft)
            .canonicalize(value)
    };
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let (Ok(left), Ok(right)) = (canonicalize(&left_source), canonicalize(&right_source)) else {
        return;
    };
    let (Ok(left_validator), Ok(right_validator)) = (build(&left_source), build(&right_source))
    else {
        return;
    };
    let pool = instance_pool();
    let verdicts: Vec<(bool, bool)> = pool
        .iter()
        .map(|instance| {
            (
                left_validator.is_valid(instance),
                right_validator.is_valid(instance),
            )
        })
        .collect();
    for (name, result, expected) in [
        (
            "union",
            left.union(&right),
            (|a: bool, b: bool| a || b) as fn(bool, bool) -> bool,
        ),
        ("intersection", left.intersect(&right), |a, b| a && b),
        ("difference", left.subtract(&right), |a, b| a && !b),
    ] {
        let Ok(result) = result else {
            continue;
        };
        let emitted = result.to_json_schema();
        let empty = result.satisfiability() == Satisfiability::No;
        let Ok(validator) = build(&emitted) else {
            continue;
        };
        for (instance, (in_left, in_right)) in pool.iter().zip(&verdicts) {
            let want = expected(*in_left, *in_right);
            assert_eq!(
                want,
                validator.is_valid(instance),
                "the {name} disagrees\n  left = {left_source}\n  right = {right_source}\n  {name} = {emitted}\n  instance = {instance}"
            );
            assert!(
                !(empty && want),
                "the {name} folded to nothing over an instance it accepts\n  left = {left_source}\n  right = {right_source}\n  instance = {instance}"
            );
        }
    }
    if let Ok(negated) = left.negate() {
        let emitted = negated.to_json_schema();
        if let Ok(validator) = build(&emitted) {
            for (instance, (in_left, _)) in pool.iter().zip(&verdicts) {
                assert_eq!(
                    !*in_left,
                    validator.is_valid(instance),
                    "the complement disagrees\n  left = {left_source}\n  complement = {emitted}\n  instance = {instance}"
                );
            }
        }
    }
    if let Ok(Containment::Yes) = left.covers(&right) {
        for (instance, (in_left, in_right)) in pool.iter().zip(&verdicts) {
            assert!(
                !in_right || *in_left,
                "covers said yes\n  left = {left_source}\n  right = {right_source}\n  instance = {instance}"
            );
        }
    }
}

/// One chained expression: how it is written, what the algebra answered, and the values it takes.
type Chain = (
    &'static str,
    Result<CanonicalSchema, jsonschema::canonical::CanonicalizationError>,
    fn(bool, bool, bool) -> bool,
);

// Chaining across three versions of one document renames a second time, over keys an earlier round
// already renamed and roots an earlier round already redirected. Whatever a chain answers holds on
// every value.
#[hegel::test(test_cases = 5_000)]
fn chained_set_operations_over_disagreeing_documents_answer_or_decline(tc: TestCase) {
    let draft = draw_draft(&tc);
    let body = match draw_schema_node(&tc, 2) {
        object @ Value::Object(_) => object,
        other => json!({ "allOf": [other] }),
    };
    let body = if tc.draw(gs::booleans()) {
        json!({"type": "object", "allOf": [body], "properties": {"self": {"$ref": "#"}}})
    } else {
        body
    };
    let document = |defs: Value| {
        let mut schema = body.clone();
        schema
            .as_object_mut()
            .expect("the body was wrapped into an object")
            .insert("$defs".into(), defs);
        schema
    };
    let sources = [
        document(shared_defs()),
        document(edited_defs(&tc, draw_leaf(&tc))),
        document(edited_defs(&tc, draw_leaf(&tc))),
    ];
    let canonicalize = |value: &Value| {
        jsonschema::canonical::options()
            .with_draft(draft)
            .canonicalize(value)
    };
    let build = |value: &Value| jsonschema::options().with_draft(draft).build(value);
    let mut operands = Vec::new();
    for source in &sources {
        let Ok(document) = canonicalize(source) else {
            return;
        };
        operands.push(draw_operand(&tc, &document));
    }
    let pool = instance_pool();
    let mut verdicts = Vec::new();
    for operand in &operands {
        // A handle on a target answers about that target, so the validator is built from what it
        // emits rather than from the document it came out of.
        let Ok(validator) = build(&operand.to_json_schema()) else {
            return;
        };
        verdicts.push(
            pool.iter()
                .map(|instance| validator.is_valid(instance))
                .collect::<Vec<bool>>(),
        );
    }
    let (a, b, c) = (&operands[0], &operands[1], &operands[2]);
    let chains: [Chain; 5] = [
        (
            "(a | b) & c",
            a.union(b).and_then(|left| left.intersect(c)),
            |a, b, c| (a || b) && c,
        ),
        (
            "(a & b) | c",
            a.intersect(b).and_then(|left| left.union(c)),
            |a, b, c| (a && b) || c,
        ),
        (
            "(a \\ b) \\ c",
            a.subtract(b).and_then(|left| left.subtract(c)),
            |a, b, c| a && !b && !c,
        ),
        (
            "a & (b | c)",
            b.union(c).and_then(|left| left.intersect(a)),
            |a, b, c| a && (b || c),
        ),
        (
            "(a | b) \\ c",
            a.union(b).and_then(|left| left.subtract(c)),
            |a, b, c| (a || b) && !c,
        ),
    ];
    for (chain, result, expected) in chains {
        let Ok(result) = result else {
            continue;
        };
        let emitted = result.to_json_schema();
        let empty = result.satisfiability() == Satisfiability::No;
        let Ok(validator) = build(&emitted) else {
            continue;
        };
        for (index, instance) in pool.iter().enumerate() {
            let want = expected(verdicts[0][index], verdicts[1][index], verdicts[2][index]);
            assert_eq!(
                want,
                validator.is_valid(instance),
                "`{chain}` disagrees\n  a = {}\n  b = {}\n  c = {}\n  {chain} = {emitted}\n  instance = {instance}",
                a.to_json_schema(),
                b.to_json_schema(),
                c.to_json_schema()
            );
            assert!(
                !(empty && want),
                "`{chain}` folded to nothing over an instance it accepts\n  a = {}\n  b = {}\n  c = {}\n  instance = {instance}",
                a.to_json_schema(),
                b.to_json_schema(),
                c.to_json_schema()
            );
        }
    }
}
