#![cfg(target_arch = "wasm32")]
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

use jsonschema_wasm::{bundle, canonicalize, dereference, meta_validate, validate};
use serde::Serialize;
use serde_json::json;
use wasm_bindgen::JsValue;

// `ms` is wall-clock timing (`js_sys::Date::now()`), so it can't be equality-checked.
// Assert it is present and numeric, then strip it so the rest of the result can be
// compared against an exact expected value.
fn without_ms(res: JsValue) -> serde_json::Value {
    let mut v: serde_json::Value = serde_wasm_bindgen::from_value(res).unwrap();
    let ms = v.get("ms").expect("result must carry ms");
    assert!(ms.is_number(), "ms must be a number");
    v.as_object_mut().unwrap().remove("ms");
    v
}

#[wasm_bindgen_test]
async fn validate_passing() {
    let res = validate(
        r#"{"type":"object","required":["id"]}"#.into(),
        r#"{"id":1}"#.into(),
        JsValue::UNDEFINED,
    )
    .await
    .unwrap();
    assert_eq!(without_ms(res), json!({"valid": true, "errors": []}));
}

#[wasm_bindgen_test]
async fn validate_failing() {
    let res = validate(
        r#"{"type":"object","required":["id"]}"#.into(),
        r#"{}"#.into(),
        JsValue::UNDEFINED,
    )
    .await
    .unwrap();
    assert_eq!(
        without_ms(res),
        json!({
            "valid": false,
            "errors": [
                {
                    "message": "\"id\" is a required property",
                    "instancePath": [],
                    "schemaPath": ["required"],
                    "kind": {"type": "required", "property": "id"}
                }
            ]
        })
    );
}

#[wasm_bindgen_test]
async fn dereference_inlines_local_ref() {
    let schema = r##"{"properties":{"a":{"$ref":"#/$defs/x"}},"$defs":{"x":{"type":"string"}}}"##;
    let res = dereference(schema.into(), JsValue::UNDEFINED)
        .await
        .unwrap();
    assert_eq!(
        without_ms(res),
        json!({
            "output": {
                "properties": {"a": {"type": "string"}},
                "$defs": {"x": {"type": "string"}}
            }
        })
    );
}

// json-schema.org/draft/ refs resolve from vendored meta-schemas (record_refs skip) and never
// reach the retriever — exercising FetchRetriever needs a URL outside that prefix. Pinned commit.
const REMOTE_SCHEMA_URL: &str = "https://raw.githubusercontent.com/Stranger6667/jsonschema/80b99eb8c699749c3b8d36ea7b6a0661e2dd217a/crates/benchmark/data/fast_schema.json";

#[wasm_bindgen_test]
async fn remote_ref_resolves_via_fetch() {
    let schema = format!(r#"{{"$ref":"{REMOTE_SCHEMA_URL}"}}"#);
    let instance = r#"[5,"hello",[1,"a",true],{"a":"x","b":"y","c":"z"},"ok",3]"#;
    let res = validate(schema, instance.into(), JsValue::UNDEFINED)
        .await
        .unwrap();
    assert_eq!(without_ms(res), json!({"valid": true, "errors": []}));
}

#[wasm_bindgen_test]
async fn bundle_embeds_remote_ref() {
    let schema = format!(r#"{{"$ref":"{REMOTE_SCHEMA_URL}"}}"#);
    let res = bundle(schema, JsValue::UNDEFINED).await.unwrap();
    assert_eq!(
        without_ms(res),
        json!({
            "output": {
                "$ref": REMOTE_SCHEMA_URL,
                "$defs": {
                    REMOTE_SCHEMA_URL: {
                        "$id": REMOTE_SCHEMA_URL,
                        "$schema": "http://json-schema.org/draft-07/schema#",
                        "type": "array",
                        "items": [
                            {"type": "number", "exclusiveMaximum": 10},
                            {"type": "string", "enum": ["hello", "world"]},
                            {
                                "type": "array",
                                "minItems": 1,
                                "maxItems": 3,
                                "items": [
                                    {"type": "number"},
                                    {"type": "string"},
                                    {"type": "boolean"}
                                ]
                            },
                            {
                                "type": "object",
                                "required": ["a", "b"],
                                "minProperties": 3,
                                "properties": {
                                    "a": {"type": ["null", "string"]},
                                    "b": {"type": ["null", "string"]},
                                    "c": {"type": ["null", "string"], "default": "abc"}
                                },
                                "additionalProperties": {"type": "string"}
                            },
                            {"not": {"type": ["null"]}},
                            {
                                "oneOf": [
                                    {"type": "number", "multipleOf": 3},
                                    {"type": "number", "multipleOf": 5}
                                ]
                            }
                        ]
                    }
                }
            }
        })
    );
}

// Reads `res.output` the way any real JS consumer (e.g. the playground, which feeds it
// straight to `JSON.stringify`) actually reads it, instead of routing back through
// `serde_wasm_bindgen::from_value` — which understands `Map` and hides the bug.
#[wasm_bindgen_test]
async fn dereference_output_is_plain_object() {
    let schema = r##"{"properties":{"a":{"$ref":"#/$defs/x"}},"$defs":{"x":{"type":"string"}}}"##;
    let res = dereference(schema.into(), JsValue::UNDEFINED)
        .await
        .unwrap();
    let output = js_sys::Reflect::get(&res, &"output".into()).unwrap();
    let json = js_sys::JSON::stringify(&output)
        .unwrap()
        .as_string()
        .unwrap();
    assert!(
        json.contains(r#""type":"string""#),
        "output did not render as a plain object: {json}"
    );
}

// Same rationale as above: reach `kind.limit` via `Reflect::get` so an arbitrary-precision
// number token can't hide behind `serde_wasm_bindgen::from_value`'s special-cased decoding.
#[wasm_bindgen_test]
async fn error_kind_limit_is_number() {
    let res = validate(r#"{"maximum":10}"#.into(), "20".into(), JsValue::UNDEFINED)
        .await
        .unwrap();
    let errors = js_sys::Reflect::get(&res, &"errors".into()).unwrap();
    let error0 = js_sys::Reflect::get(&errors, &0u32.into()).unwrap();
    let kind = js_sys::Reflect::get(&error0, &"kind".into()).unwrap();
    let limit = js_sys::Reflect::get(&kind, &"limit".into()).unwrap();
    assert_eq!(limit.as_f64(), Some(10.0), "limit was not a JS number");
}

#[wasm_bindgen_test]
fn meta_validate_valid_schema_reports_no_errors() {
    let res = meta_validate(
        r#"{"type":"string","maxLength":5}"#.into(),
        JsValue::UNDEFINED,
    )
    .unwrap();
    assert_eq!(without_ms(res), json!({"valid": true, "errors": []}));
}

#[wasm_bindgen_test]
fn meta_validate_invalid_schema_reports_errors_pointing_into_schema() {
    let res = meta_validate(r#"{"type":123}"#.into(), JsValue::UNDEFINED).unwrap();
    let v = without_ms(res);
    assert_eq!(v["valid"], json!(false));
    let errors = v["errors"].as_array().expect("errors must be an array");
    assert!(!errors.is_empty());
    assert_eq!(errors[0]["instancePath"], json!(["type"]));
}

#[wasm_bindgen_test]
fn meta_validate_selected_draft_forces_matching_metaschema() {
    // `exclusiveMinimum: true` (boolean) is valid under Draft 4's meta-schema but must be
    // numeric under Draft 2020-12's — the selected draft must override auto-detection.
    let schema = r#"{"minimum":1,"exclusiveMinimum":true}"#;

    let draft4_opts = json!({"draft": "draft4"})
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap();
    let res = meta_validate(schema.into(), draft4_opts).unwrap();
    assert_eq!(without_ms(res), json!({"valid": true, "errors": []}));

    let draft202012_opts = json!({"draft": "draft2020-12"})
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap();
    let res = meta_validate(schema.into(), draft202012_opts).unwrap();
    assert_eq!(without_ms(res)["valid"], json!(false));
}

#[wasm_bindgen_test]
fn meta_validate_own_schema_field_wins_over_selected_draft() {
    // The schema declares Draft 2020-12 itself; the selected draft (Draft 4, where
    // `exclusiveMinimum` must be boolean) must not override that declaration.
    let schema = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "minimum": 1,
        "exclusiveMinimum": true
    }"#;
    let opts = json!({"draft": "draft4"})
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap();
    let res = meta_validate(schema.into(), opts).unwrap();
    assert_eq!(without_ms(res)["valid"], json!(false));
}

#[wasm_bindgen_test]
fn meta_validate_unknown_draft_rejects() {
    let opts = json!({"draft": "not-a-real-draft"})
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap();
    let err = meta_validate("{}".into(), opts).expect_err("unknown draft id must reject");
    assert_eq!(
        err.as_string(),
        Some("unknown draft `not-a-real-draft`".to_string())
    );
}

#[wasm_bindgen_test]
fn canonicalize_reduces_schema() {
    let schema = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "allOf": [
            {"type": "integer", "minimum": 0},
            {"minimum": 2, "maximum": 10}
        ]
    }"#;
    let res = canonicalize(schema.into(), JsValue::UNDEFINED).unwrap();
    assert_eq!(
        without_ms(res),
        json!({
            "output": {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "integer",
                "minimum": 2,
                "maximum": 10
            }
        })
    );
}

#[wasm_bindgen_test]
fn canonicalize_selected_draft_sets_output_dialect() {
    let opts = json!({"draft": "draft7"})
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap();
    let res = canonicalize(r#"{"type":"integer","minimum":0}"#.into(), opts).unwrap();
    assert_eq!(
        without_ms(res),
        json!({
            "output": {
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "integer",
                "minimum": 0
            }
        })
    );
}

#[wasm_bindgen_test]
fn canonicalize_invalid_schema_rejects() {
    let err = canonicalize(r#"{"type":123}"#.into(), JsValue::UNDEFINED)
        .expect_err("schema failing its metaschema must reject");
    assert_eq!(
        err.as_string(),
        Some(
            "schema validation failed: 123 is not valid under any of the schemas listed in the 'anyOf' keyword"
                .to_string()
        )
    );
}

#[wasm_bindgen_test]
fn canonicalize_unknown_draft_rejects() {
    let opts = json!({"draft": "not-a-real-draft"})
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap();
    let err = canonicalize("{}".into(), opts).expect_err("unknown draft id must reject");
    assert_eq!(
        err.as_string(),
        Some("unknown draft `not-a-real-draft`".to_string())
    );
}

#[wasm_bindgen_test]
async fn validate_unknown_draft_rejects() {
    // Struct fields deserialize via property access, so opts must be a plain JS object
    // (`json_compatible`), not the Map that plain `to_value` produces for JSON objects.
    let opts = json!({"draft": "not-a-real-draft"})
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap();
    let res = validate("{}".into(), "{}".into(), opts).await;
    let err = res.expect_err("unknown draft id must reject, not silently default");
    assert_eq!(
        err.as_string(),
        Some("unknown draft `not-a-real-draft`".to_string())
    );
}
