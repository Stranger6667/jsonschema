#![cfg(feature = "jsonb")]
// The encoder mirrors Postgres varlena/JEntry bit widths; the cast here is to a known-small field.
#![allow(clippy::cast_possible_truncation)]

use jsonschema::{
    json::{cmp::equal, Array, JsonNumber, Jsonb, JsonbNode, Node, Object},
    Draft, JsonType,
};
use jsonschema_value::jsonb_encode::{
    assemble, encode, encode_array_of, encode_numeric, encode_numeric_text_array,
    encode_numeric_text_object, encode_numeric_text_scalar, encode_raw_key_object, NumericForm,
};
use serde_json::{json, Map, Value};
use test_case::test_case;

// A flat (one-level) object member value.
enum Scalar<'a> {
    Int(i16),
    Str(&'a str),
    /// A container entry whose declared length overruns the buffer, so reading it panics. Only
    /// code that navigates into this member's value ever does.
    PoisonContainer,
}

// The testkit encodes any `Value`; only the overrunning entry has to be built by hand.
fn encode_object(members: &[(&str, Scalar<'_>)]) -> Vec<u8> {
    if !members
        .iter()
        .any(|(_, value)| matches!(value, Scalar::PoisonContainer))
    {
        let mut object = Map::new();
        for (key, value) in members {
            let value = match value {
                Scalar::Int(number) => json!(number),
                Scalar::Str(text) => json!(text),
                Scalar::PoisonContainer => unreachable!("checked above"),
            };
            object.insert((*key).to_string(), value);
        }
        return encode(&Value::Object(object));
    }

    let mut pairs: Vec<&(&str, Scalar<'_>)> = members.iter().collect();
    pairs.sort_by_key(|(key, _)| (key.len(), *key));
    let mut entries = Vec::new();
    let mut data = Vec::new();
    let mut total = 0_u32;
    for (index, (key, _)) in pairs.iter().enumerate() {
        data.extend_from_slice(key.as_bytes());
        let entry = key.len() as u32;
        total += entry;
        entries.push(if index == 0 {
            entry | 0x8000_0000
        } else {
            entry
        });
    }
    for (index, (_, value)) in pairs.iter().enumerate() {
        let entry = match value {
            Scalar::Str(text) => {
                data.extend_from_slice(text.as_bytes());
                text.len() as u32
            }
            Scalar::Int(number) => {
                let pad = (4 - data.len() % 4) % 4;
                data.resize(data.len() + pad, 0);
                let numeric = encode_numeric(&number.to_string(), NumericForm::Long);
                data.extend_from_slice(&numeric);
                0x1000_0000 | (pad + numeric.len()) as u32
            }
            Scalar::PoisonContainer => 0x5000_0000 | 0x000F_4240,
        };
        total += entry & 0x0FFF_FFFF;
        let index = index + pairs.len();
        entries.push(if index % 32 == 0 {
            (entry & 0x7000_0000) | total | 0x8000_0000
        } else {
            entry
        });
    }
    assemble(0x2000_0000 | pairs.len() as u32, &entries, &data)
}

// `digit * 10000^weight`, past `f64`'s range, as the decimal text the encoder takes.
fn wide_text(digit: i16, weight: i16) -> String {
    format!("{digit}e{}", i32::from(weight) * 4)
}

fn encode_wide_scalar(digit: i16, weight: i16) -> Vec<u8> {
    encode_numeric_text_scalar(&wide_text(digit, weight), NumericForm::Long)
}

fn encode_wide_object(key: &str, digit: i16, weight: i16) -> Vec<u8> {
    encode_numeric_text_object(key, &wide_text(digit, weight), NumericForm::Long)
}

fn encode_wide_array(items: &[(i16, i16)]) -> Vec<u8> {
    let texts: Vec<String> = items
        .iter()
        .map(|&(digit, weight)| wide_text(digit, weight))
        .collect();
    let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
    encode_numeric_text_array(&borrowed, NumericForm::Long)
}

// `as_str()`'s decimal expansion for a single base-10000 digit at `weight`: the digit itself,
// unpadded, followed by `weight` groups of four zeros for the unset lower digits.
fn digit_at_weight(digit: i16, weight: i16) -> String {
    let weight = usize::try_from(weight).expect("weight is non-negative");
    format!("{digit}{}", "0".repeat(4 * weight))
}

// Every number embedded in `node`, depth-first: pins the fixture's own encoded magnitude to the
// digits its constructor named, so a change to the encoders or to `as_str` cannot go unnoticed.
fn embedded_numbers(node: &JsonbNode<'_>) -> Vec<String> {
    if let Some(number) = node.as_number() {
        return vec![number.as_str().into_owned()];
    }
    if let Some(array) = node.as_array() {
        return array
            .elements()
            .flat_map(|element| embedded_numbers(&element))
            .collect();
    }
    if let Some(object) = node.as_object() {
        return object
            .members()
            .flat_map(|(_, value)| embedded_numbers(&value))
            .collect();
    }
    Vec::new()
}

// `serde_json::Number` cannot hold this at all, so `enum`/`const`/`uniqueItems` have to compare
// through `JsonNumber`.

#[test_case(
    json!({"enum": [0, 1]}),
    encode_wide_scalar(1, 100),
    false,
    JsonType::Number,
    vec![digit_at_weight(1, 100)];
    "enum rejects a number beyond f64"
)]
#[test_case(
    json!({"const": {"big": 0}}),
    encode_wide_object("big", 1, 100),
    false,
    JsonType::Object,
    vec![digit_at_weight(1, 100)];
    "const rejects a number beyond f64"
)]
#[test_case(
    json!({"uniqueItems": true}),
    encode_wide_array(&[(1, 100), (2, 100)]),
    true,
    JsonType::Array,
    vec![digit_at_weight(1, 100), digit_at_weight(2, 100)];
    "uniqueItems tells apart two numbers beyond f64"
)]
#[allow(clippy::needless_pass_by_value)]
fn wrong_verdicts_on_numbers_beyond_f64(
    schema: Value,
    instance: Vec<u8>,
    expected: bool,
    expected_type: JsonType,
    expected_numbers: Vec<String>,
) {
    let root = Jsonb::root(&instance);
    assert_eq!(root.json_type(), expected_type);
    assert_eq!(embedded_numbers(&root), expected_numbers);

    let validator = jsonschema::options_for::<Jsonb>()
        .build(&schema)
        .expect("schema builds");
    assert_eq!(validator.is_valid(root), expected);
}

#[test]
fn validates_a_jsonb_instance() {
    let validator = jsonschema::options_for::<Jsonb>()
        .build(&json!({"type": "integer", "minimum": 18}))
        .expect("schema builds");

    let ok = encode(&json!(21));
    assert!(validator.is_valid(Jsonb::root(&ok)));

    let bad = encode(&json!(7));
    assert!(!validator.is_valid(Jsonb::root(&bad)));
}

// A bad capture would silently poison every benchmark built on it. The captures are
// little-endian, and the benchmarks reading them never run big-endian.
#[cfg_attr(target_endian = "big", ignore = "little-endian captures")]
#[test_case(benchmark::FHIR_PATIENT, benchmark::FHIR_PATIENT_JSONB; "fhir patient")]
#[test_case(
    benchmark::FHIR_PATIENT_INVALID,
    benchmark::FHIR_PATIENT_INVALID_JSONB;
    "fhir patient invalid"
)]
#[test_case(benchmark::FAST_VALID, benchmark::FAST_VALID_JSONB; "fast valid")]
#[test_case(benchmark::FAST_INVALID, benchmark::FAST_INVALID_JSONB; "fast invalid")]
#[test_case(benchmark::GEOJSON, benchmark::GEOJSON_INSTANCE_JSONB; "geojson schema as instance")]
#[test_case(
    benchmark::GEOJSON_INSTANCE_INVALID,
    benchmark::GEOJSON_INSTANCE_INVALID_JSONB;
    "geojson schema as instance, invalid"
)]
fn captured_fixture_round_trips(json_bytes: &[u8], jsonb_bytes: &[u8]) {
    let expected: Value = serde_json::from_slice(json_bytes).expect("fixture is valid JSON");
    let decoded = Jsonb::root(jsonb_bytes).to_value();
    assert!(
        equal(&decoded, &expected),
        "captured jsonb bytes decode to {decoded:?}, expected {expected:?}"
    );
}

// A `jsonb`-backed error defers building its instance; a `serde_json`-backed one already has it.
// Laziness changes when the instance is built, never what the error looks like.
#[test_case(
    json!({"required": ["name"]}),
    json!({}),
    encode_object(&[]);
    "required: object is missing the property"
)]
#[test_case(
    json!({"properties": {"age": {"type": "number"}}}),
    json!({"age": "oops"}),
    encode_object(&[("age", Scalar::Str("oops"))]);
    "type: a property has the wrong type"
)]
#[test_case(
    json!({"properties": {"name": {"minLength": 3}}}),
    json!({"name": "hi"}),
    encode_object(&[("name", Scalar::Str("hi"))]);
    "minLength: a property is too short"
)]
#[test_case(
    json!({"additionalProperties": false}),
    json!({"extra": 1}),
    encode_object(&[("extra", Scalar::Int(1))]);
    "additionalProperties: an undeclared property is present"
)]
#[allow(clippy::needless_pass_by_value)]
fn instance_matches_across_representations(schema: Value, instance: Value, jsonb: Vec<u8>) {
    let serde_validator = jsonschema::validator_for(&schema).expect("schema builds");
    let serde_error = serde_validator
        .validate(&instance)
        .expect_err("instance is invalid");

    let jsonb_validator = jsonschema::options_for::<Jsonb>()
        .build(&schema)
        .expect("schema builds");
    let jsonb_error = jsonb_validator
        .validate(Jsonb::root(&jsonb))
        .expect_err("instance is invalid");

    assert_eq!(
        jsonb_error.instance().as_ref(),
        serde_error.instance().as_ref()
    );
    assert_eq!(jsonb_error.to_string(), serde_error.to_string());
    assert_eq!(
        format!("{:?}", jsonb_error.kind()),
        format!("{:?}", serde_error.kind())
    );
    assert_eq!(jsonb_error.instance_path(), serde_error.instance_path());
    assert_eq!(jsonb_error.schema_path(), serde_error.schema_path());
}

// `instance()` builds the value once and returns the same storage on every later call: the
// `OnceLock` inside `LazyInstance` memoizes rather than rebuilding.
#[test]
fn jsonb_instance_memoizes_rather_than_rebuilding() {
    let schema = json!({"required": ["name"]});
    let validator = jsonschema::options_for::<Jsonb>()
        .build(&schema)
        .expect("schema builds");
    let jsonb = encode_object(&[]);
    let error = validator
        .validate(Jsonb::root(&jsonb))
        .expect_err("instance is invalid");

    let first: *const Value = error.instance().as_ref();
    let second: *const Value = error.instance().as_ref();
    assert!(std::ptr::eq(first, second));
}

// Errors are routinely collected before anything inspects them, so the deferred instance has to
// survive that move.
#[test]
fn jsonb_errors_resolve_their_instance_after_being_collected() {
    let schema = json!({
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "number"}
        },
        "required": ["name", "age"]
    });
    let validator = jsonschema::options_for::<Jsonb>()
        .build(&schema)
        .expect("schema builds");
    let jsonb = encode_object(&[("age", Scalar::Str("oops"))]);
    let errors: Vec<_> = validator.iter_errors(Jsonb::root(&jsonb)).collect();
    assert_eq!(errors.len(), 2, "missing `name` and wrong-typed `age`");

    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert!(messages.contains(&"\"name\" is a required property".to_string()));
    assert!(messages.contains(&"\"oops\" is not of type \"number\"".to_string()));
}

// Reading `poison`'s value panics, and a `required` miss on a third key never navigates into any
// member. Building the instance eagerly at error construction would panic here.
#[test]
fn required_miss_never_materializes_a_poisoned_sibling() {
    let schema = json!({"required": ["missing"]});
    let validator = jsonschema::options_for::<Jsonb>()
        .build(&schema)
        .expect("schema builds");
    let jsonb = encode_object(&[
        ("ok", Scalar::Str("x")),
        ("poison", Scalar::PoisonContainer),
    ]);
    let error = validator
        .validate(Jsonb::root(&jsonb))
        .expect_err("\"missing\" is absent");

    assert_eq!(error.to_string(), "\"missing\" is a required property");
    assert_eq!(
        format!("{:?}", error.kind()),
        "Required { property: String(\"missing\") }"
    );
}

// Sanity check for the fixture above: `poison` really does panic once something reads it, so the
// test above is proof the instance was never touched, not a coincidence.
#[test]
#[should_panic = "out of range"]
fn poisoned_sibling_panics_once_actually_read() {
    let jsonb = encode_object(&[
        ("ok", Scalar::Str("x")),
        ("poison", Scalar::PoisonContainer),
    ]);
    let _ = Jsonb::root(&jsonb).to_value();
}

// `type: integer` alone and `type: [integer, ...]` reach different checks, so a `numeric` whose
// fraction is lost in an `f64` must not be an integer to one and not the other.
#[test_case(&json!({"type": "integer"}); "integer alone")]
#[test_case(&json!({"type": ["integer", "string"]}); "integer among several")]
fn a_lost_fraction_is_not_an_integer(schema: &Value) {
    let instance = encode_numeric_text_scalar("10000000000000000.5", NumericForm::Long);
    let validator = jsonschema::options_for::<Jsonb>()
        .build(schema)
        .expect("schema builds");
    assert!(!validator.is_valid(Jsonb::root(&instance)));
}

// Draft 4 excludes anything written with a decimal point from `type: integer`, however zero the
// fraction is; `jsonb` keeps that in the numeric's scale.
#[test_case("1.0", false; "zero fraction is not a draft 4 integer")]
#[test_case("1", true; "no point is a draft 4 integer")]
fn draft4_integer_follows_how_the_number_was_written(text: &str, expected: bool) {
    let instance = encode_numeric_text_scalar(text, NumericForm::Long);
    let validator = jsonschema::options_for::<Jsonb>()
        .with_draft(Draft::Draft4)
        .build(&json!({"type": "integer"}))
        .expect("schema builds");
    assert_eq!(validator.is_valid(Jsonb::root(&instance)), expected);
}

// A `numeric` past `f64` must not answer keywords as if it were infinity.
#[test_case(&json!({"multipleOf": 1}), true; "every integer is a multiple of one")]
#[test_case(&json!({"multipleOf": 2}), true; "a power of ten is a multiple of two")]
#[test_case(&json!({"multipleOf": 3}), false; "a power of ten is not a multiple of three")]
#[test_case(&json!({"minimum": 0}), true; "above the minimum")]
#[test_case(&json!({"maximum": 0}), false; "not below the maximum")]
fn keywords_agree_on_a_number_beyond_f64(schema: &Value, expected: bool) {
    let instance = encode_numeric_text_scalar("1e400", NumericForm::Long);
    let validator = jsonschema::options_for::<Jsonb>()
        .build(schema)
        .expect("schema builds");
    assert_eq!(validator.is_valid(Jsonb::root(&instance)), expected);
}

// The benchmark fixtures are far larger than any suite instance, so they exercise the cycle guard
// and the `is_valid` memoization over a deeply repeated document.
#[test_case(benchmark::FHIR_SCHEMA, benchmark::FHIR_PATIENT, benchmark::FHIR_PATIENT_JSONB; "fhir patient")]
#[test_case(
    benchmark::FHIR_SCHEMA,
    benchmark::FHIR_PATIENT_INVALID,
    benchmark::FHIR_PATIENT_INVALID_JSONB;
    "fhir patient invalid"
)]
#[test_case(benchmark::FAST_SCHEMA, benchmark::FAST_VALID, benchmark::FAST_VALID_JSONB; "fast valid")]
#[test_case(benchmark::FAST_SCHEMA, benchmark::FAST_INVALID, benchmark::FAST_INVALID_JSONB; "fast invalid")]
#[cfg_attr(target_endian = "big", ignore = "little-endian captures")]
fn captured_fixture_verdicts_match(schema: &[u8], json_bytes: &[u8], jsonb_bytes: &[u8]) {
    let schema: Value = serde_json::from_slice(schema).expect("schema is valid JSON");
    let instance: Value = serde_json::from_slice(json_bytes).expect("fixture is valid JSON");

    let with_serde = jsonschema::validator_for(&schema)
        .expect("schema builds")
        .is_valid(&instance);
    let with_jsonb = jsonschema::options_for::<Jsonb>()
        .build(&schema)
        .expect("schema builds")
        .is_valid(Jsonb::root(jsonb_bytes));
    assert_eq!(with_jsonb, with_serde);
}

#[test]
fn mutual_reference_cycle_over_an_empty_scalar_terminates() {
    let schema = json!({
        "$defs": {
            "a": {"$ref": "#/$defs/b"},
            "b": {"$ref": "#/$defs/a"}
        },
        "$ref": "#/$defs/a"
    });
    let validator = jsonschema::options_for::<Jsonb>()
        .build(&schema)
        .expect("schema builds");
    let instance = encode(&json!(""));
    assert!(validator.is_valid(Jsonb::root(&instance)));
}

// A key that is not UTF-8 decodes to the replacement character, which would fold distinct keys
// together; equality reads the stored bytes instead.
#[test_case(&[0xff], &[0xff], false; "the same invalid key twice is a duplicate")]
#[test_case(&[0xff], &[0xfe], true; "two different invalid keys are not")]
#[test_case("\u{fffd}".as_bytes(), &[0xff], true; "a replacement character is not the byte it stands for")]
fn objects_compare_keys_by_stored_bytes(left: &[u8], right: &[u8], unique: bool) {
    let instance = encode_array_of(&[
        encode_raw_key_object(left, true),
        encode_raw_key_object(right, true),
    ]);
    let validator = jsonschema::options_for::<Jsonb>()
        .build(&json!({"uniqueItems": true}))
        .expect("schema builds");
    assert_eq!(validator.is_valid(Jsonb::root(&instance)), unique);
}
