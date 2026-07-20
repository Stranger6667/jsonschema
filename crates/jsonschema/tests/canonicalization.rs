use std::cmp::Ordering;

use jsonschema::{
    canonical::{options, CanonicalKind, CanonicalSchema, CanonicalView},
    canonicalize, Draft, JsonType,
};
use serde_json::{json, Map, Value};
use test_case::test_case;

#[test_case(&json!({"type": "string", "minLength": 3}); "string constraints")]
#[test_case(&json!({"allOf": [{"type": "integer"}, {"minimum": 0}]}); "allOf")]
#[test_case(&json!({"$defs": {"a": {"type": "null"}}, "$ref": "#/$defs/a"}); "ref into defs")]
fn unmodeled_document_round_trips_verbatim(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(&canonical.to_json_schema(), schema);
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
}

#[test_case(&json!({"type": "string"}), CanonicalKind::MultiType, "multi_type"; "multi_type")]
#[test_case(&json!({"const": 1}), CanonicalKind::Const, "const"; "const_value")]
#[test_case(&json!({"enum": [1, 2, 3]}), CanonicalKind::Enum, "enum"; "enum_values")]
#[test_case(&json!({}), CanonicalKind::True, "true"; "empty object")]
#[test_case(&json!(false), CanonicalKind::False, "false"; "boolean false")]
#[test_case(&json!({"type": "string", "minLength": 3}), CanonicalKind::Raw, "raw"; "raw")]
fn kind_reports_its_label(schema: &Value, kind: CanonicalKind, label: &str) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(canonical.kind(), kind);
    assert_eq!(canonical.kind().as_str(), label);
}

// `view()` exposes each modeled node with its payload.
#[test_case(&json!({"type": ["string", "number"]}), &CanonicalView::MultiType(JsonType::String | JsonType::Number); "multi_type")]
#[test_case(&json!({"const": [1, "a"]}), &CanonicalView::Const(json!([1, "a"])); "const_value")]
#[test_case(&json!({"enum": [3, 1, 2]}), &CanonicalView::Enum(vec![json!(1), json!(2), json!(3)]); "enum_values")]
#[test_case(&json!(true), &CanonicalView::True; "boolean true")]
#[test_case(&json!({}), &CanonicalView::True; "empty object")]
#[test_case(&json!(false), &CanonicalView::False; "boolean false")]
fn view_matches_expected(schema: &Value, expected: &CanonicalView) {
    assert_eq!(&canonicalize(schema).unwrap().view(), expected);
}

// Draft 4 keeps a type guard on `integer` values because value equality cannot tell `1` from `1.0`.
#[test]
fn draft4_integer_values_are_a_typed_group() {
    let canonical = options()
        .with_draft(Draft::Draft4)
        .canonicalize(&json!({"type": "integer", "enum": [1, 2, 3]}))
        .expect("canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::TypedGroup);
    assert_eq!(canonical.kind().as_str(), "typed_group");
    let CanonicalView::TypedGroup(group) = canonical.view() else {
        panic!("expected a TypedGroup view");
    };
    assert_eq!(group.ty, JsonType::Integer);
    assert_eq!(group.body.kind(), CanonicalKind::Enum);
}

// An `anyOf` whose branches stay disjoint surfaces as an AnyOf view exposing each branch.
#[test]
fn view_exposes_anyof_branches() {
    let canonical =
        canonicalize(&json!({"anyOf": [{"type": "string"}, {"const": 1}]})).expect("canonicalizes");
    assert_eq!(canonical.kind(), CanonicalKind::AnyOf);
    assert_eq!(canonical.kind().as_str(), "any_of");
    let CanonicalView::AnyOf(branches) = canonical.view() else {
        panic!("expected an AnyOf view");
    };
    assert_eq!(
        branches
            .iter()
            .map(CanonicalSchema::view)
            .collect::<Vec<_>>(),
        vec![
            CanonicalView::MultiType(JsonType::String.into()),
            CanonicalView::Const(json!(1)),
        ]
    );
}

#[test]
fn validation_error_display_and_source() {
    let error = canonicalize(&json!({"type": 123})).expect_err("invalid schema must error");
    assert!(error.to_string().starts_with("schema validation failed:"));
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn deeply_nested_document_round_trips() {
    let mut schema = json!({"type": "string"});
    for _ in 0..300 {
        let mut map = Map::new();
        map.insert("not".to_string(), schema);
        schema = Value::Object(map);
    }
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert_eq!(canonical.to_json_schema(), schema);
}

// Numerals `ext::numeric::try_parse_bigint` refuses (huge exponents / digit counts) have no
// exact runtime comparison; documents carrying them in `const`/`enum` stay raw.
#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"const":1e999999999999999999999}"#; "huge_exponent_const")]
#[test_case(r#"{"enum":[1e999999999999999999999]}"#; "huge_exponent_enum")]
#[test_case(&format!(r#"{{"const":1{}}}"#, "0".repeat((1 << 20) + 1)); "huge_digit_count")]
fn numerals_without_exact_comparison_stay_raw(text: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
    assert_eq!(canonical.to_json_schema(), schema);
}

// `const` compares by JSON value, so `1` and `1.0` share one canonical form; distinct values stay distinct.
#[test]
fn const_identity_is_value_identity() {
    let integer = canonicalize(&json!({"const": 1})).unwrap();
    let float = canonicalize(&json!({"const": 1.0})).unwrap();
    assert_eq!(integer, float);
    assert_ne!(integer, canonicalize(&json!({"const": "1"})).unwrap());
    assert_eq!(
        integer.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "const": 1})
    );
}

// An integer-valued float folds to its integer form on both sides of zero.
#[test_case(&json!(5.0), &json!(5); "positive")]
#[test_case(&json!(-5.0), &json!(-5); "negative")]
fn integer_valued_float_const_folds_to_integer(float: &Value, integer: &Value) {
    let from_float = canonicalize(&json!({ "const": float })).unwrap();
    let from_integer = canonicalize(&json!({ "const": integer })).unwrap();
    assert_eq!(from_float, from_integer);
    assert_eq!(
        from_float.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "const": integer})
    );
}

// A finite value set that fills a JSON type's whole domain collapses to a `type`; a partial set stays an `enum`.
#[test_case(&json!({"enum": [null, false, true]}), &json!({"type": ["null", "boolean"]}); "saturates null and boolean")]
#[test_case(&json!({"enum": [false, true]}), &json!({"type": "boolean"}); "saturates boolean")]
#[test_case(&json!({"enum": [null, false]}), &json!({"enum": [null, false]}); "partial set stays enum")]
fn finite_value_set_saturation(schema: &Value, expected: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    assert_eq!(canonical.to_json_schema(), Value::Object(expected));
}

// `const` and `enum` together admit only the values in both.
#[test]
fn const_intersects_enum() {
    let canonical = canonicalize(&json!({"enum": [1, 2, 3], "const": 2})).expect("canonicalizes");
    assert_eq!(
        canonical.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "const": 2})
    );
}

// `CanonicalSchema` orders structurally: a schema equals itself and differs from a distinct one.
#[test]
fn canonical_schema_ordering() {
    let one = canonicalize(&json!({"const": 1})).unwrap();
    let two = canonicalize(&json!({"const": 2})).unwrap();
    assert_eq!(one.cmp(&one), Ordering::Equal);
    assert!(one < two);
    assert!(two > one);
}

// Each draft stamps its own `$schema` URI onto the emitted document.
#[test_case(Draft::Draft6, "http://json-schema.org/draft-06/schema#"; "draft6")]
#[test_case(Draft::Draft201909, "https://json-schema.org/draft/2019-09/schema"; "draft2019-09")]
fn draft_stamps_its_schema_uri(draft: Draft, uri: &str) {
    let canonical = options()
        .with_draft(draft)
        .canonicalize(&json!({"type": "string"}))
        .expect("canonicalizes");
    assert_eq!(
        canonical.to_json_schema(),
        json!({"$schema": uri, "type": "string"})
    );
}
