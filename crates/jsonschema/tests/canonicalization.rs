use jsonschema::canonical::{options, CanonicalView};
use serde_json::{json, Map, Value};
use test_case::test_case;

#[test_case(&json!({"type": "string", "minLength": 3}); "string constraints")]
#[test_case(&json!({"allOf": [{"type": "integer"}, {"minimum": 0}]}); "allOf")]
#[test_case(&json!({"$defs": {"a": {"type": "null"}}, "$ref": "#/$defs/a"}); "ref into defs")]
#[test_case(&json!({}); "empty object")]
#[test_case(&json!(true); "boolean true")]
#[test_case(&json!(false); "boolean false")]
fn every_document_round_trips_verbatim(schema: &Value) {
    let canonical = options().canonicalize(schema).expect("canonicalizes");
    assert_eq!(&canonical.to_json_schema(), schema);
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
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

#[test]
fn numerically_equal_but_distinct_documents_stay_distinct() {
    let integer = options().canonicalize(&json!({"const": 1})).unwrap();
    let float = options().canonicalize(&json!({"const": 1.0})).unwrap();
    assert_ne!(integer, float);
    assert_eq!(
        integer,
        options().canonicalize(&json!({"const": 1})).unwrap()
    );
    assert_eq!(integer.to_json_schema(), json!({"const": 1}));
    assert_eq!(float.to_json_schema(), json!({"const": 1.0}));
}
