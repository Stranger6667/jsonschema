use jsonschema::canonical::{options, CanonicalKind, CanonicalView};
use serde_json::{json, Map, Value};
use test_case::test_case;

#[test_case(&json!({"type": "string", "minLength": 3}); "string constraints")]
#[test_case(&json!({"allOf": [{"type": "integer"}, {"minimum": 0}]}); "allOf")]
#[test_case(&json!({"$defs": {"a": {"type": "null"}}, "$ref": "#/$defs/a"}); "ref into defs")]
fn unmodeled_document_round_trips_verbatim(schema: &Value) {
    let canonical = options().canonicalize(schema).expect("canonicalizes");
    assert_eq!(&canonical.to_json_schema(), schema);
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
}

#[test_case(&json!({}), CanonicalKind::True; "empty object")]
#[test_case(&json!(true), CanonicalKind::True; "boolean true")]
#[test_case(&json!(false), CanonicalKind::False; "boolean false")]
fn trivial_documents_model_structurally(schema: &Value, expected: CanonicalKind) {
    let canonical = options().canonicalize(schema).expect("canonicalizes");
    assert_eq!(canonical.kind(), expected);
}

#[test]
fn validation_error_display_and_source() {
    let error =
        jsonschema::canonicalize(&json!({"type": 123})).expect_err("invalid schema must error");
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
    let canonical = options().canonicalize(&schema).expect("canonicalizes");
    assert_eq!(canonical.to_json_schema(), schema);
}

// Numerals `ext::numeric::try_parse_bigint` refuses (huge exponents / digit counts) have no
// exact runtime comparison; documents carrying them in `const`/`enum` stay raw.
#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"const":1e999999999999999999999}"#; "huge_exponent")]
#[test_case(&format!(r#"{{"const":1{}}}"#, "0".repeat((1 << 20) + 1)); "huge_digit_count")]
fn numerals_without_exact_comparison_stay_raw(text: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let canonical = options().canonicalize(&schema).expect("canonicalizes");
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
    assert_eq!(canonical.to_json_schema(), schema);
}

// `const` compares by JSON value, so `1` and `1.0` share one canonical form; distinct values stay distinct.
#[test]
fn const_identity_is_value_identity() {
    let integer = options().canonicalize(&json!({"const": 1})).unwrap();
    let float = options().canonicalize(&json!({"const": 1.0})).unwrap();
    assert_eq!(integer, float);
    assert_ne!(
        integer,
        options().canonicalize(&json!({"const": "1"})).unwrap()
    );
    assert_eq!(
        integer.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "const": 1})
    );
}
