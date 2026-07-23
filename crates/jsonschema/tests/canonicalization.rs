use std::{cmp::Ordering, collections::HashSet};

use jsonschema::{
    canonical::{options, CanonicalKind, CanonicalSchema, CanonicalView},
    canonicalize, CanonicalizationError, Draft, JsonType, PatternOptions,
};
use serde_json::{json, Map, Number, Value};
use test_case::test_case;

#[test_case(&json!({"patternProperties": {"^a$": {"type": "string"}}}); "pattern properties")]
#[test_case(&json!({"$defs": {"a": {"type": "null"}}, "$ref": "#/$defs/a"}); "ref into defs")]
fn unmodeled_document_round_trips_verbatim(schema: &Value) {
    let canonical = canonicalize(schema).expect("canonicalizes");
    assert_eq!(&canonical.to_json_schema(), schema);
    assert!(matches!(canonical.view(), CanonicalView::Raw(_)));
}

#[test]
fn string_view_exposes_facets() {
    let CanonicalView::String(view) =
        canonicalize(&json!({"type": "string", "minLength": 2, "pattern": "^a"}))
            .unwrap()
            .view()
    else {
        panic!("expected a String view");
    };
    assert_eq!(view.min_length, Some(Number::from(2u64)));
    assert_eq!(view.max_length, None);
    assert_eq!(view.patterns, vec!["^a".to_string()]);
}

#[test]
fn integer_view_exposes_bounds() {
    let CanonicalView::Integer(view) =
        canonicalize(&json!({"type": "integer", "minimum": 2, "maximum": 9}))
            .unwrap()
            .view()
    else {
        panic!("expected an Integer view");
    };
    assert_eq!(view.minimum, Some(Number::from(2)));
    assert_eq!(view.maximum, Some(Number::from(9)));
}

#[test]
fn array_view_exposes_bounds() {
    let CanonicalView::Array(view) =
        canonicalize(&json!({"type": "array", "minItems": 1, "maxItems": 3, "uniqueItems": true}))
            .unwrap()
            .view()
    else {
        panic!("expected an Array view");
    };
    assert_eq!(view.min_items, Some(Number::from(1u64)));
    assert_eq!(view.max_items, Some(Number::from(3u64)));
    assert!(view.unique_items);
}

#[test]
fn object_view_exposes_bounds() {
    let CanonicalView::Object(view) = canonicalize(
        &json!({"type": "object", "minProperties": 1, "maxProperties": 3, "required": ["a"]}),
    )
    .unwrap()
    .view() else {
        panic!("expected an Object view");
    };
    // A required key already demands the one property `minProperties` asked for.
    assert_eq!(view.min_properties, None);
    assert_eq!(view.max_properties, Some(Number::from(3u64)));
    assert_eq!(view.required, vec!["a".to_string()]);
    assert!(view.property_names.is_none());
    assert!(view.properties.is_empty());
}

#[test]
fn object_view_exposes_properties() {
    let CanonicalView::Object(view) =
        canonicalize(&json!({"type": "object", "properties": {"a": {"type": "integer"}}}))
            .unwrap()
            .view()
    else {
        panic!("expected an Object view");
    };
    let schema = view.properties.get("a").expect("a property schema");
    assert_eq!(
        schema.to_json_schema(),
        json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer"})
    );
}

#[test]
fn object_view_exposes_property_names() {
    let CanonicalView::Object(view) =
        canonicalize(&json!({"type": "object", "propertyNames": {"maxLength": 4}}))
            .unwrap()
            .view()
    else {
        panic!("expected an Object view");
    };
    let names = view.property_names.expect("a key constraint");
    assert_eq!(
        names.to_json_schema(),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "string",
            "maxLength": 4
        })
    );
}

// A format the canonicalizer cannot check may still be checked at validation, so a union must not
// absorb a value into a leaf carrying one.
#[test_case(&json!({"type": "object", "propertyNames": {"format": "only-ok"}}), &json!({"nope": 1}); "key constraint")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "string", "format": "only-ok"}}}), &json!({"a": "nope"}); "property schema")]
#[test_case(&json!({"type": "object", "properties": {"a": {"format": "only-ok"}}}), &json!({"a": "nope"}); "untyped property schema")]
#[test_case(&json!({"type": "object", "properties": {"a": {"type": "object", "propertyNames": {"format": "only-ok"}}}}), &json!({"a": {"nope": 1}}); "nested object")]
fn uncheckable_format_keeps_the_value_beside_the_leaf(leaf: &Value, instance: &Value) {
    let schema = json!({"anyOf": [{"const": instance}, leaf]});
    let canonical = options()
        .should_validate_formats(true)
        .canonicalize(&schema)
        .expect("canonicalizes");
    let build = |value: &Value| {
        ::jsonschema::options()
            .with_format("only-ok", |text: &str| text == "ok")
            .should_validate_formats(true)
            .build(value)
            .expect("builds")
    };
    assert!(build(&schema).is_valid(instance));
    assert!(build(&canonical.to_json_schema()).is_valid(instance));
}

// A Draft 4 integer property schema is a typed group, which the format scan walks past to reach the
// key whose format it cannot check.
#[test]
fn uncheckable_format_scan_walks_a_typed_group() {
    let instance = json!({"b": "nope"});
    let schema = json!({"anyOf": [
        {"enum": [instance]},
        {"type": "object", "properties": {
            "a": {"type": "integer", "enum": [1, 2]},
            "b": {"type": "string", "format": "only-ok"}
        }}
    ]});
    let canonical = options()
        .with_draft(Draft::Draft4)
        .should_validate_formats(true)
        .canonicalize(&schema)
        .expect("canonicalizes");
    let build = |value: &Value| {
        ::jsonschema::options()
            .with_draft(Draft::Draft4)
            .with_format("only-ok", |text: &str| text == "ok")
            .should_validate_formats(true)
            .build(value)
            .expect("builds")
    };
    assert!(build(&schema).is_valid(&instance));
    assert!(build(&canonical.to_json_schema()).is_valid(&instance));
}

// An unmodeled document keeps document identity, where `1` and `1.0` are distinct - unlike JSON
// value equality, which reads them as the same number.
#[test]
fn unmodeled_documents_hash_by_document_identity() {
    let canonical = |text: &str| {
        canonicalize(&serde_json::from_str::<Value>(text).expect("valid schema JSON"))
            .expect("canonicalizes")
    };
    let integer = canonical(
        r#"{"patternProperties": {"^a$": {"enum": [1, null, true, "x", [2], {"b": 3}]}}}"#,
    );
    let float = canonical(
        r#"{"patternProperties": {"^a$": {"enum": [1.0, null, true, "x", [2], {"b": 3}]}}}"#,
    );
    assert_eq!(integer.kind(), CanonicalKind::Raw);
    let distinct: HashSet<CanonicalSchema> =
        [integer.clone(), float, integer].into_iter().collect();
    assert_eq!(distinct.len(), 2);
}

#[test]
fn number_view_exposes_bounds() {
    let CanonicalView::Number(view) = canonicalize(&json!({
        "type": "number",
        "exclusiveMinimum": 1.5,
        "maximum": 9.5
    }))
    .unwrap()
    .view() else {
        panic!("expected a Number view");
    };
    assert_eq!(view.minimum, Number::from_f64(1.5));
    assert!(view.exclusive_minimum);
    assert_eq!(view.maximum, Number::from_f64(9.5));
    assert!(!view.exclusive_maximum);
}

// Arbitrary precision models a bound past `u64`/`i64` as a big integer and emits it back exactly;
// the default build keeps such a document raw.
#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"type": "string", "minLength": 99999999999999999999999}"#, CanonicalKind::String, "minLength"; "length bound")]
#[test_case(r#"{"type": "integer", "minimum": 99999999999999999999999}"#, CanonicalKind::Integer, "minimum"; "integer bound")]
#[test_case(r#"{"type": "array", "minItems": 99999999999999999999999}"#, CanonicalKind::Array, "minItems"; "array length bound")]
#[test_case(r#"{"type": "object", "minProperties": 99999999999999999999999}"#, CanonicalKind::Object, "minProperties"; "object size bound")]
fn past_range_bound_round_trips(text: &str, kind: CanonicalKind, keyword: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert_eq!(canonical.kind(), kind);
    assert_eq!(
        canonical.to_json_schema()[keyword].to_string(),
        "99999999999999999999999"
    );
}

#[cfg(not(feature = "arbitrary-precision"))]
#[test_case("string", "minLength")]
#[test_case("string", "maxLength")]
#[test_case("array", "minItems")]
#[test_case("array", "maxItems")]
#[test_case("object", "minProperties")]
#[test_case("object", "maxProperties")]
fn huge_count_bound_stays_raw(ty: &str, keyword: &str) {
    let schema: Value = serde_json::from_str(&format!(
        r#"{{"type": "{ty}", "{keyword}": 99999999999999999999999}}"#
    ))
    .unwrap();
    assert_eq!(canonicalize(&schema).unwrap().kind(), CanonicalKind::Raw);
}

// Default build: the integers past `i64` that such a bound admits have no modeled form. They still
// satisfy the schema, so the document stays raw rather than collapsing to "nothing matches". A
// `number` interval carries the same bound, and an `allOf` may put it against `integer` later.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(r#"{"type": "integer", "minimum": 99999999999999999999999}"#; "integer minimum")]
#[test_case(r#"{"type": "integer", "maximum": 99999999999999999999999}"#; "integer maximum")]
#[test_case(r#"{"type": "number", "minimum": 99999999999999999999999}"#; "number minimum")]
#[test_case(r#"{"type": "number", "maximum": 99999999999999999999999}"#; "number maximum")]
#[test_case(r#"{"allOf": [{"type": "integer"}, {"minimum": 99999999999999999999999}]}"#; "interval meeting integer")]
fn huge_numeric_bound_stays_raw(text: &str) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    assert_eq!(canonicalize(&schema).unwrap().kind(), CanonicalKind::Raw);
}

// The `regex` engine rejects a negative lookahead the fancy engine accepts.
#[test]
fn pattern_engine_selects_dialect() {
    let schema = json!({"pattern": "^(?!x)"});
    assert!(canonicalize(&schema).is_ok());
    let error = options()
        .with_pattern_options(PatternOptions::regex())
        .canonicalize(&schema)
        .unwrap_err();
    assert!(matches!(
        error,
        CanonicalizationError::InvalidPattern { .. }
    ));
}

// The suite checks only the error variant; the `Display` message is exercised here.
#[test_case(&json!(42), "schema must be a boolean or object, got: 42"; "invalid schema type")]
#[test_case(&json!({"pattern": "["}), "invalid regular expression: \"[\""; "invalid pattern")]
fn error_display(schema: &Value, message: &str) {
    assert_eq!(canonicalize(schema).unwrap_err().to_string(), message);
}

#[test_case(&json!({"type": "string"}), CanonicalKind::MultiType, "multi_type"; "multi_type")]
#[test_case(&json!({"const": 1}), CanonicalKind::Const, "const"; "const_value")]
#[test_case(&json!({"enum": [1, 2, 3]}), CanonicalKind::Enum, "enum"; "enum_values")]
#[test_case(&json!({}), CanonicalKind::True, "true"; "empty object")]
#[test_case(&json!(false), CanonicalKind::False, "false"; "boolean false")]
#[test_case(&json!({"type": "integer", "minimum": 0}), CanonicalKind::Integer, "integer"; "integer_leaf")]
#[test_case(&json!({"type": "number", "minimum": 0}), CanonicalKind::Number, "number"; "number_leaf")]
#[test_case(&json!({"patternProperties": {"^a$": {"type": "string"}}}), CanonicalKind::Raw, "raw"; "raw")]
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

// Default build: an integer past `i64` lies above the whole representable range, so it satisfies any
// minimum and violates any maximum - decided by overflow direction, without representing it.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(r#"{"type":"integer","enum":[1,2,10000000000000000000],"minimum":2}"#, &json!({"enum":[2,10_000_000_000_000_000_000_u64]}); "minimum keeps oversized")]
#[test_case(r#"{"type":"integer","enum":[1,2,10000000000000000000],"maximum":5}"#, &json!({"enum":[1,2]}); "maximum drops oversized")]
#[test_case(r#"{"allOf":[{"type":"integer","minimum":2},{"enum":[1,2,10000000000000000000]}]}"#, &json!({"enum":[2,10_000_000_000_000_000_000_u64]}); "cross-branch minimum keeps oversized")]
#[test_case(r#"{"type":"integer","enum":[1,2,-10000000000000000000],"maximum":5}"#, &json!({"enum":[1,2,-1e19]}); "maximum keeps undersized")]
#[test_case(r#"{"type":"integer","enum":[1,2,-10000000000000000000],"minimum":0}"#, &json!({"enum":[1,2]}); "minimum drops undersized")]
fn oversized_integer_compares_by_overflow_direction(text: &str, expected: &Value) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let canonical = canonicalize(&schema).expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    assert_eq!(canonical.to_json_schema(), Value::Object(expected));
}

// Default build: a value past `i64` cannot lift into a window, so a covering interval absorbs it by
// overflow direction alone.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(r#"{"anyOf":[{"type":"integer","minimum":2},{"const":1e30}]}"#, CanonicalKind::Integer; "absorbed above every maximum")]
#[test_case(r#"{"anyOf":[{"type":"integer","maximum":5},{"const":1e30}]}"#, CanonicalKind::AnyOf; "kept beyond the maximum")]
fn oversized_member_absorption(text: &str, kind: CanonicalKind) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    assert_eq!(canonicalize(&schema).expect("canonicalizes").kind(), kind);
}

// Draft 4 `integer` is a typed group an interval bound narrows; a bound excluding every member leaves
// nothing satisfiable, a mixed type set guards only its integer members, and the bound may sit on
// either side of the intersection.
#[test_case(&json!({"type": "integer", "enum": [1, 2, 3], "minimum": 2}), &json!({"type": "integer", "enum": [2, 3]}); "narrows to survivors")]
#[test_case(&json!({"type": "integer", "enum": [1, 2, 3], "minimum": 5}), &json!({"not": {}}); "bound excludes all")]
#[test_case(&json!({"allOf": [{"type": ["string", "integer"]}, {"enum": ["a", 1]}]}), &json!({"anyOf": [{"type": "integer", "enum": [1]}, {"enum": ["a"]}]}); "mixed type set guards only integers")]
#[test_case(&json!({"allOf": [{"type": "integer", "minimum": 2}, {"type": "integer", "enum": [1, 2, 3]}]}), &json!({"type": "integer", "enum": [2, 3]}); "bound before typed group")]
fn draft4_integer_typed_group_intersects_bound(schema: &Value, expected: &Value) {
    let canonical = options()
        .with_draft(Draft::Draft4)
        .canonicalize(schema)
        .expect("canonicalizes");
    let mut expected = expected.as_object().expect("object").clone();
    expected.insert(
        "$schema".into(),
        json!("http://json-schema.org/draft-04/schema#"),
    );
    assert_eq!(canonical.to_json_schema(), Value::Object(expected));
}

// Draft 4 keeps a type guard on `integer` values because value equality cannot tell `1` from `1.0`,
// whether the values come from the same object or meet a bound from another `allOf` branch.
#[test_case(&json!({"type": "integer", "enum": [1, 2, 3]}); "same object")]
#[test_case(&json!({"allOf": [{"enum": [1, 2, 3]}, {"type": "integer", "minimum": 2}]}); "value set meets a bound")]
fn draft4_integer_values_are_a_typed_group(schema: &Value) {
    let canonical = options()
        .with_draft(Draft::Draft4)
        .canonicalize(schema)
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

// Past `f64` precision a whole divisor keeps exact modulo only under arbitrary precision, so the
// forms below are default-build behaviour.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(
    r#"{"type": "integer", "multipleOf": 9007199254740993}"#,
    &json!({"type": "integer", "multipleOf": 9_007_199_254_740_993_u64});
    "a divisor no decimal spells is kept as written"
)]
#[test_case(
    r#"{"type": "integer", "multipleOf": 4611686018427387903}"#,
    &json!({"type": "integer", "multipleOf": 4_611_686_018_427_387_903_u64});
    "a divisor past f64 precision is kept as written"
)]
#[test_case(
    r#"{"type": "integer", "multipleOf": 1e30}"#,
    &json!({"type": "integer", "multipleOf": 1e30});
    "a divisor past the integer range is kept as written"
)]
#[test_case(
    r#"{"allOf":[{"type":"integer","multipleOf":9007199254740992},{"type":"integer","multipleOf":9007199254740991}]}"#,
    &json!({"type": "integer", "allOf": [{"multipleOf": 9_007_199_254_740_991_u64}, {"multipleOf": 9_007_199_254_740_992_u64}]});
    "divisors with no exact common multiple stay apart"
)]
#[test_case(
    r#"{"allOf":[{"type":"integer","multipleOf":9007199254740992,"minimum":1},{"type":"integer","multipleOf":9007199254740991}]}"#,
    &json!({"type": "integer", "minimum": 9_007_199_254_740_992_u64, "allOf": [{"multipleOf": 9_007_199_254_740_991_u64}, {"multipleOf": 9_007_199_254_740_992_u64}]});
    "divisors with no exact common multiple keep a snapped bound"
)]
#[test_case(
    r#"{"allOf":[{"type":"number","multipleOf":3},{"type":"number","multipleOf":3002399751580331}]}"#,
    &json!({"type": "integer", "allOf": [{"multipleOf": 3}, {"multipleOf": 3_002_399_751_580_331_u64}]});
    "divisors whose common multiple no decimal spells stay apart"
)]
fn divisors_past_exact_precision(text: &str, expected: &Value) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let mut form = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    assert_eq!(&form, expected);
}

// A member the divisor admits survives even where the integer type cannot hold it.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn divisor_keeps_member_past_representable_range() {
    let schema = json!({"allOf": [{"type": "integer", "multipleOf": 2}, {"const": 1e30}]});
    let mut form = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    assert_eq!(form, json!({"const": 1e30}));
}

// Membership for a divisor is decided by the validator's own arithmetic, so every rewrite the
// algebra makes rests on this agreeing with a compiled `multipleOf`.
#[test_case("2")]
#[test_case("3")]
#[test_case("1")]
#[test_case("0.5")]
#[test_case("0.75")]
#[test_case("1.5")]
#[test_case("0.123456789")]
#[test_case("9007199254740992")]
#[test_case("9007199254740993")]
#[test_case("4503599627370496")]
#[test_case("1e300")]
#[test_case("1e-7")]
fn divisor_oracle_matches_the_validator(divisor: &str) {
    const INSTANCES: &[&str] = &[
        "0",
        "1",
        "2",
        "3",
        "6",
        "-4",
        "1.5",
        "2.5",
        "0.25",
        "9007199254740993",
        "12345678900000001",
        "27021597764222977",
        "1e30",
        "-9007199254740993",
    ];
    let divisor: serde_json::Number = divisor.parse().expect("divisor");
    let validator = jsonschema::validator_for(&json!({"multipleOf": divisor})).expect("compiles");
    for instance in INSTANCES {
        let instance: serde_json::Number = instance.parse().expect("instance");
        assert_eq!(
            jsonschema_value::numeric_check::satisfies_multiple_of(&divisor, &instance),
            validator.is_valid(&Value::Number(instance.clone())),
            "multipleOf {divisor} on {instance}"
        );
    }
}

// A divisor no `f64` spells still constrains, so the leaf carries it instead of the document staying
// raw; only the arithmetic that would need its exact value is skipped.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn divisor_no_decimal_spells_is_modeled() {
    let schema = json!({"type": "number", "multipleOf": 9_007_199_254_740_993_u64});
    let canonical = canonicalize(&schema).expect("canonicalizes");
    assert_ne!(canonical.kind(), jsonschema::canonical::CanonicalKind::Raw);
    let mut form = canonical.to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    // The validator reads the divisor as 2^53, whose multiples are all whole.
    assert_eq!(
        form,
        json!({"type": "integer", "multipleOf": 9_007_199_254_740_993_u64})
    );
}

// Bounds past `f64` precision: snapping must not move an end onto a value the validator reads
// differently, and a progression whose next multiple is unrepresentable is not empty.
#[cfg(not(feature = "arbitrary-precision"))]
#[test_case(
    r#"{"type":"number","minimum":9223372036854775807,"multipleOf":1}"#,
    &["9223372036854775807", "9223372036854775808"];
    "a bound with no representable multiple"
)]
#[test_case(
    r#"{"type":"number","minimum":-4,"maximum":9223372036854775807,"multipleOf":0.5}"#,
    &["9223372036854775808", "-4", "0.5"];
    "an upper end past exact precision"
)]
#[test_case(
    r#"{"type":"number","exclusiveMinimum":9007199254740992,"multipleOf":0.5}"#,
    &["9007199254740992", "9007199254740993"];
    "an excluded end past exact precision"
)]
#[test_case(
    r#"{"type":"integer","minimum":9223372036854775807,"multipleOf":2}"#,
    &["9223372036854775808", "1e30", "9223372036854775807"];
    "an integer bound with no representable multiple"
)]
fn wide_bounds_keep_validation(text: &str, instances: &[&str]) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let emitted = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    for instance in instances {
        let instance: Value = serde_json::from_str(instance).expect("instance");
        assert_eq!(
            jsonschema::is_valid(&schema, &instance),
            jsonschema::is_valid(&emitted, &instance),
            "{instance} against {emitted}"
        );
    }
}

// A divisor of one adds nothing beside a whole one, whose multiples are already whole. The wide
// divisor keeps its spelling only in the default build.
#[cfg(not(feature = "arbitrary-precision"))]
#[test]
fn identity_divisor_drops_beside_a_whole_one() {
    let schema = json!({"allOf": [
        {"type": "number", "multipleOf": 2},
        {"type": "number", "minimum": 0, "multipleOf": 1e30}
    ]});
    let mut form = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    assert_eq!(
        form,
        json!({"type": "integer", "minimum": 0, "multipleOf": 1e30})
    );
}

// Arbitrary precision decides every divisor exactly, so divisors the default build reads with
// different arithmetic still fold there.
#[cfg(feature = "arbitrary-precision")]
#[test_case(
    r#"{"allOf":[{"type":"number","multipleOf":3},{"type":"number","multipleOf":1.5}]}"#,
    &json!({"type": "integer", "multipleOf": 3});
    "a whole divisor stands for a fractional one it covers"
)]
#[test_case(
    r#"{"allOf":[{"type":"number","multipleOf":2},{"type":"number","multipleOf":2.5}]}"#,
    &json!({"type": "integer", "multipleOf": 10});
    "unlike divisors fold to their common multiple"
)]
fn unlike_divisors_fold_under_arbitrary_precision(text: &str, expected: &Value) {
    let schema: Value = serde_json::from_str(text).expect("valid schema JSON");
    let mut form = canonicalize(&schema)
        .expect("canonicalizes")
        .to_json_schema();
    form.as_object_mut().expect("object").remove("$schema");
    assert_eq!(&form, expected);
}
