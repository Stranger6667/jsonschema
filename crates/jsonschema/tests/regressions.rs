use jsonschema::error::ValidationErrorKind;
use serde_json::json;

#[test]
fn recursive_ref_preserves_unevaluated_properties() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$id": "https://example.com/root",
        "$recursiveAnchor": true,
        "type": "object",
        "properties": {
            "child": {
                "type": "object",
                "properties": {
                    "child": { "$recursiveRef": "#" }
                },
                "unevaluatedProperties": false
            }
        },
        "unevaluatedProperties": false
    });

    let validator = jsonschema::options()
        .build(&schema)
        .expect("schema compiles");

    let valid = json!({"child": {"child": {}}});
    assert!(
        validator.is_valid(&valid),
        "expected recursive schema without extras to be valid"
    );

    let invalid = json!({"child": {"child": {"unexpected": 1}}});
    assert!(
        !validator.is_valid(&invalid),
        "unexpected properties should be rejected"
    );

    let errors: Vec<_> = validator.iter_errors(&invalid).collect();
    assert!(
        errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::UnevaluatedProperties { .. })),
        "expected unevaluatedProperties error, got {errors:?}"
    );
}
