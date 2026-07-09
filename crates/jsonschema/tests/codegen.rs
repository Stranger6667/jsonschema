#![allow(clippy::needless_pass_by_value)]
use test_case::test_case;

fn is_currency_format(value: &str) -> bool {
    value
        .strip_prefix('$')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
}

fn is_literal_x(value: &str) -> bool {
    value == "x"
}

struct EvenKeyword;

impl jsonschema::Keyword for EvenKeyword {
    fn validate<'i>(
        &self,
        instance: &'i serde_json::Value,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(jsonschema::ValidationError::custom("number must be even"))
        }
    }

    fn is_valid(&self, instance: &serde_json::Value) -> bool {
        instance.as_u64().is_none_or(|n| n % 2 == 0)
    }
}

fn even_factory<'a>(
    _parent: &'a serde_json::Map<String, serde_json::Value>,
    value: &'a serde_json::Value,
    _path: jsonschema::paths::Location,
) -> Result<Box<dyn jsonschema::Keyword>, jsonschema::ValidationError<'a>> {
    if value.as_bool() == Some(true) {
        Ok(Box::new(EvenKeyword))
    } else {
        Err(jsonschema::ValidationError::custom("`even` must be true"))
    }
}

struct MultiErrorKeyword;

impl jsonschema::Keyword for MultiErrorKeyword {
    fn validate<'i>(
        &self,
        instance: &'i serde_json::Value,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(jsonschema::ValidationError::custom("first"))
        }
    }

    fn is_valid(&self, instance: &serde_json::Value) -> bool {
        instance.as_i64().is_none_or(|n| n > 100)
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i serde_json::Value,
    ) -> Box<dyn Iterator<Item = jsonschema::ValidationError<'i>> + 'i> {
        if self.is_valid(instance) {
            Box::new(std::iter::empty())
        } else {
            Box::new(
                vec![
                    jsonschema::ValidationError::custom("first"),
                    jsonschema::ValidationError::custom("second"),
                ]
                .into_iter(),
            )
        }
    }
}

// The Result wrapping is required by the keyword factory signature.
#[allow(clippy::unnecessary_wraps)]
fn multi_error_factory<'a>(
    _parent: &'a serde_json::Map<String, serde_json::Value>,
    _value: &'a serde_json::Value,
    _path: jsonschema::paths::Location,
) -> Result<Box<dyn jsonschema::Keyword>, jsonschema::ValidationError<'a>> {
    Ok(Box::new(MultiErrorKeyword))
}

fn is_hex_content(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn check_prefixed(value: &str) -> bool {
    value.starts_with("p:")
}

// The Result/Option wrapping is required by the content-encoding converter signature.
#[allow(clippy::unnecessary_wraps)]
fn convert_prefixed(value: &str) -> Result<Option<String>, jsonschema::ValidationError<'static>> {
    Ok(value.strip_prefix("p:").map(str::to_string))
}

fn assert_validate_parity(
    generated_is_valid: bool,
    generated: Result<(), jsonschema::ValidationError<'_>>,
    runtime: &jsonschema::Validator,
    instance: &serde_json::Value,
) {
    assert_eq!(generated_is_valid, runtime.is_valid(instance));
    match (generated, runtime.validate(instance)) {
        (Ok(()), Ok(())) => {}
        (Err(generated), Err(expected)) => {
            assert_eq!(generated.to_string(), expected.to_string());
            assert_eq!(generated.schema_path(), expected.schema_path());
            assert_eq!(generated.instance_path(), expected.instance_path());
        }
        (generated, expected) => {
            panic!("validate() parity mismatch: generated={generated:?}, runtime={expected:?}")
        }
    }
}

// Builds the default runtime validator from `schema` and defers to `assert_validate_parity`.
fn assert_validate_parity_for(
    schema: &serde_json::Value,
    generated_is_valid: bool,
    generated: Result<(), jsonschema::ValidationError<'_>>,
    instance: &serde_json::Value,
) {
    let runtime = jsonschema::validator_for(schema).expect("valid schema");
    assert_validate_parity(generated_is_valid, generated, &runtime, instance);
}

// Asserts generated `iter_errors()` matches `runtime.iter_errors()` on message + paths, in order.
fn assert_iter_errors_parity<'i>(
    generated: impl Iterator<Item = jsonschema::ValidationError<'i>>,
    runtime: &jsonschema::Validator,
    instance: &serde_json::Value,
) {
    let generated: Vec<_> = generated.collect();
    let expected: Vec<_> = runtime.iter_errors(instance).collect();
    assert_eq!(
        generated.len(),
        expected.len(),
        "iter_errors count mismatch: generated={:?}, runtime={:?}",
        generated
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        expected.iter().map(ToString::to_string).collect::<Vec<_>>()
    );
    for (generated, expected) in generated.iter().zip(&expected) {
        assert_eq!(generated.to_string(), expected.to_string());
        assert_eq!(generated.schema_path(), expected.schema_path());
        assert_eq!(generated.instance_path(), expected.instance_path());
    }
}

// Asserts that the generated `is_valid` result agrees with the default runtime
// validator built from `schema`.
fn assert_is_valid_parity(
    schema: &serde_json::Value,
    generated_is_valid: bool,
    instance: &serde_json::Value,
) {
    let runtime = jsonschema::validator_for(schema).expect("valid schema");
    assert_eq!(
        generated_is_valid,
        runtime.is_valid(instance),
        "codegen/runtime mismatch for {instance}"
    );
}

#[cfg(feature = "arbitrary-precision")]
fn runtime_valid(schema: &serde_json::Value, instance: &serde_json::Value) -> bool {
    jsonschema::validator_for(schema)
        .expect("valid schema")
        .is_valid(instance)
}

fn build_runtime_with_resources(
    schema: serde_json::Value,
    resources: impl IntoIterator<Item = (&'static str, serde_json::Value)>,
) -> jsonschema::Validator {
    let resources: Vec<_> = resources.into_iter().collect();
    let mut builder = jsonschema::Registry::new();
    for (uri, resource) in &resources {
        builder = builder.add(*uri, resource).expect("resource accepted");
    }
    let registry = builder.prepare().expect("registry build failed");
    jsonschema::options()
        .with_registry(&registry)
        .build(&schema)
        .expect("schema should build with custom vocabulary resources")
}

#[jsonschema::validator(
    schema = r#"{"multi":true}"#,
    keywords = { "multi" => crate::multi_error_factory }
)]
struct MultiErrorCustomValidator;

#[test]
fn test_custom_keyword_iter_errors_multi() {
    let schema = serde_json::json!({"multi": true});
    let instance = serde_json::json!(1);
    let runtime = jsonschema::options()
        .with_keyword("multi", multi_error_factory)
        .build(&schema)
        .expect("valid schema");
    assert_iter_errors_parity(
        MultiErrorCustomValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"integer","even":true}"#,
    keywords = { "even" => crate::even_factory }
)]
struct CustomKeywordValidator;

#[test_case(serde_json::json!(4) ; "even_integer")]
#[test_case(serde_json::json!(3) ; "odd_integer")]
#[test_case(serde_json::json!("x") ; "non_integer")]
fn test_custom_keyword_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"integer","even":true});
    let runtime = jsonschema::options()
        .with_keyword("even", even_factory)
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        CustomKeywordValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
    match (
        CustomKeywordValidator::validate(&instance),
        runtime.validate(&instance),
    ) {
        (Ok(()), Ok(())) => {}
        (Err(generated), Err(expected)) => {
            assert_eq!(generated.to_string(), expected.to_string());
            assert_eq!(generated.schema_path(), expected.schema_path());
            assert_eq!(generated.instance_path(), expected.instance_path());
        }
        (generated, expected) => {
            panic!("validate() parity mismatch: generated={generated:?}, runtime={expected:?}")
        }
    }
}

#[jsonschema::validator(
    schema = r#"{"contentMediaType":"text/hex"}"#,
    draft = Draft7,
    content_media_types = { "text/hex" => crate::is_hex_content }
)]
struct CustomContentMediaTypeValidator;

#[jsonschema::validator(
    schema = r#"{"contentEncoding":"prefixed"}"#,
    draft = Draft7,
    content_encodings = { "prefixed" => { check = crate::check_prefixed, convert = crate::convert_prefixed } }
)]
struct CustomContentEncodingValidator;

#[jsonschema::validator(
    schema = r#"{"contentEncoding":"prefixed","contentMediaType":"application/json"}"#,
    draft = Draft7,
    content_encodings = { "prefixed" => { check = crate::check_prefixed, convert = crate::convert_prefixed } }
)]
struct CustomEncodingBuiltinMediaValidator;

#[jsonschema::validator(schema = r#"{"contentEncoding":"base64"}"#, draft = Draft7)]
struct BuiltinContentEncodingValidator;

#[jsonschema::validator(schema = r#"{"contentEncoding":"BASE64"}"#, draft = Draft7)]
struct UppercaseContentEncodingValidator;

#[test_case(serde_json::json!("deadBEEF") ; "valid_hex")]
#[test_case(serde_json::json!("xyz") ; "invalid_hex")]
#[test_case(serde_json::json!(5) ; "non_string")]
fn test_custom_content_media_type_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .with_content_media_type("text/hex", is_hex_content)
        .build(&serde_json::json!({"contentMediaType":"text/hex"}))
        .expect("valid schema");
    assert_validate_parity(
        CustomContentMediaTypeValidator::is_valid(&instance),
        CustomContentMediaTypeValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

#[test_case(serde_json::json!("p:payload") ; "valid_prefixed")]
#[test_case(serde_json::json!("payload") ; "missing_prefix")]
fn test_custom_content_encoding_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .with_content_encoding("prefixed", check_prefixed, convert_prefixed)
        .build(&serde_json::json!({"contentEncoding":"prefixed"}))
        .expect("valid schema");
    assert_validate_parity(
        CustomContentEncodingValidator::is_valid(&instance),
        CustomContentEncodingValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

#[test_case(serde_json::json!("p:{\"a\":1}") ; "decodes_to_json")]
#[test_case(serde_json::json!("p:not json") ; "decodes_to_non_json")]
#[test_case(serde_json::json!("no prefix") ; "conversion_fails")]
fn test_custom_encoding_with_builtin_media_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .with_content_encoding("prefixed", check_prefixed, convert_prefixed)
        .build(&serde_json::json!({"contentEncoding":"prefixed","contentMediaType":"application/json"}))
        .expect("valid schema");
    assert_validate_parity(
        CustomEncodingBuiltinMediaValidator::is_valid(&instance),
        CustomEncodingBuiltinMediaValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

#[test_case(serde_json::json!("aGVsbG8=") ; "valid_base64")]
#[test_case(serde_json::json!("!!!") ; "invalid_base64")]
fn test_builtin_content_encoding_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&serde_json::json!({"contentEncoding":"base64"}))
        .expect("valid schema");
    assert_validate_parity(
        BuiltinContentEncodingValidator::is_valid(&instance),
        BuiltinContentEncodingValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

// Content names are case-sensitive: `BASE64` is an unknown encoding and
// validates nothing, exactly like the runtime validator.
#[test]
fn test_unknown_uppercase_content_encoding_is_ignored() {
    let instance = serde_json::json!("!!!");
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&serde_json::json!({"contentEncoding":"BASE64"}))
        .expect("valid schema");
    assert!(runtime.is_valid(&instance));
    assert!(UppercaseContentEncodingValidator::is_valid(&instance));
}

// Regex-engine arm (no prefix/exact/alternation optimization).
#[jsonschema::validator(schema = r#"{"type":"string","pattern":"^ab+$"}"#)]
struct PatternRegexParityValidator;

// Prefix optimization must still report the original pattern in the error.
#[jsonschema::validator(schema = r#"{"type":"string","pattern":"^abc"}"#)]
struct PatternPrefixParityValidator;

#[test]
fn test_pattern_validate_parity() {
    let schema = serde_json::json!({"type":"string","pattern":"^ab+$"});
    let instance = serde_json::json!("xyz");
    assert_validate_parity_for(
        &schema,
        PatternRegexParityValidator::is_valid(&instance),
        PatternRegexParityValidator::validate(&instance),
        &instance,
    );

    // Prefix-optimized check must still report the original pattern `^abc`.
    let schema = serde_json::json!({"type":"string","pattern":"^abc"});
    let instance = serde_json::json!("xyz");
    assert_validate_parity_for(
        &schema,
        PatternPrefixParityValidator::is_valid(&instance),
        PatternPrefixParityValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(schema = r#"{"type":"string","format":"ipv4"}"#, draft = referencing::Draft::Draft7, validate_formats = true)]
struct FormatBuiltinParityValidator;

#[test]
fn test_format_validate_parity() {
    // custom format (Draft7 validates formats by default)
    let schema = serde_json::json!({"type":"string","format":"currency"});
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .with_format("currency", is_currency_format)
        .should_validate_formats(true)
        .build(&schema)
        .expect("valid schema");
    let instance = serde_json::json!("nope");
    assert_validate_parity(
        CustomFormatDraft7Validator::is_valid(&instance),
        CustomFormatDraft7Validator::validate(&instance),
        &runtime,
        &instance,
    );

    // builtin format
    let schema = serde_json::json!({"type":"string","format":"ipv4"});
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .should_validate_formats(true)
        .build(&schema)
        .expect("valid schema");
    let instance = serde_json::json!("not-an-ip");
    assert_validate_parity(
        FormatBuiltinParityValidator::is_valid(&instance),
        FormatBuiltinParityValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

#[test]
fn test_unevaluated_properties_validate_parity() {
    // Plain `unevaluatedProperties: false` (offender via allOf-tracked properties).
    let schema = serde_json::json!({"type":"object","allOf":[{"properties":{"a":{}}}],"unevaluatedProperties":false});
    let instance = serde_json::json!({"a":1,"b":2});
    assert_validate_parity_for(
        &schema,
        UnevalPropsAllOfValidator::is_valid(&instance),
        UnevalPropsAllOfValidator::validate(&instance),
        &instance,
    );

    // `unevaluatedProperties` as a schema: offender is the key whose value fails the schema.
    let schema = serde_json::json!({"type":"object","properties":{"a":{}},"unevaluatedProperties":{"type":"integer"}});
    let instance = serde_json::json!({"a":1,"b":"x"});
    assert_validate_parity_for(
        &schema,
        UnevalPropsSchemaValidator::is_valid(&instance),
        UnevalPropsSchemaValidator::validate(&instance),
        &instance,
    );

    // Self-referential `$dynamicRef` cycle (regression for the cycle fix).
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"node","type":"object","$dynamicRef":"#node","unevaluatedProperties":false});
    let instance = serde_json::json!({"a":1});
    assert_validate_parity_for(
        &schema,
        DynamicAnchorCycleUnevalPropsValidator::is_valid(&instance),
        DynamicAnchorCycleUnevalPropsValidator::validate(&instance),
        &instance,
    );
}

#[test]
fn test_unevaluated_items_validate_parity() {
    let schema = serde_json::json!({"type":"array","allOf":[{"prefixItems":[{"type":"integer"}]}],"unevaluatedItems":false});
    let instance = serde_json::json!([1, 2]);
    assert_validate_parity_for(
        &schema,
        UnevalItemsAllOfValidator::is_valid(&instance),
        UnevalItemsAllOfValidator::validate(&instance),
        &instance,
    );

    // Self-referential `$dynamicRef` cycle (regression for the cycle fix).
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"node","type":"array","$dynamicRef":"#node","unevaluatedItems":false});
    let instance = serde_json::json!([1]);
    assert_validate_parity_for(
        &schema,
        DynamicAnchorCycleUnevalItemsValidator::is_valid(&instance),
        DynamicAnchorCycleUnevalItemsValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(schema = r#"{"propertyNames":{"pattern":"^a+$"}}"#)]
struct PropertyNamesPatternParityValidator;

#[jsonschema::validator(schema = r#"{"propertyNames":{"maxLength":3}}"#)]
struct PropertyNamesMaxLengthParityValidator;

#[test]
fn test_property_names_validate_parity() {
    let schema = serde_json::json!({"propertyNames":{"pattern":"^a+$"}});
    let instance = serde_json::json!({"aaA": {}});
    assert_validate_parity_for(
        &schema,
        PropertyNamesPatternParityValidator::is_valid(&instance),
        PropertyNamesPatternParityValidator::validate(&instance),
        &instance,
    );

    let schema = serde_json::json!({"propertyNames":{"maxLength":3}});
    let instance = serde_json::json!({"foobar": {}});
    assert_validate_parity_for(
        &schema,
        PropertyNamesMaxLengthParityValidator::is_valid(&instance),
        PropertyNamesMaxLengthParityValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(schema = r#"{"propertyNames":false}"#)]
struct PropertyNamesFalseParityValidator;

#[test]
fn test_property_names_false_validate_parity() {
    let schema = serde_json::json!({"propertyNames":false});
    let instance = serde_json::json!({"foo":"bar"});
    assert_validate_parity_for(
        &schema,
        PropertyNamesFalseParityValidator::is_valid(&instance),
        PropertyNamesFalseParityValidator::validate(&instance),
        &instance,
    );
    // empty object is valid
    assert!(PropertyNamesFalseParityValidator::is_valid(
        &serde_json::json!({})
    ));
    assert!(PropertyNamesFalseParityValidator::validate(&serde_json::json!({})).is_ok());
}

#[jsonschema::validator(
    schema = r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"array","items":[{"type":"integer"}],"additionalItems":false}"#
)]
struct AdditionalItemsFalseParityValidator;

#[test]
fn test_additional_items_validate_parity() {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2019-09/schema","type":"array","items":[{"type":"integer"}],"additionalItems":false});
    let instance = serde_json::json!([1, 2, "x"]);
    assert_validate_parity_for(
        &schema,
        AdditionalItemsFalseParityValidator::is_valid(&instance),
        AdditionalItemsFalseParityValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{}},"additionalProperties":false}"#
)]
struct AdditionalPropsFalseParityValidator;

#[test]
fn test_additional_properties_false_validate_parity() {
    // properties + additionalProperties:false, no patternProperties (known-keys precheck)
    let schema =
        serde_json::json!({"type":"object","properties":{"a":{}},"additionalProperties":false});
    let instance = serde_json::json!({"a":1,"b":2,"c":3});
    assert_validate_parity_for(
        &schema,
        AdditionalPropsFalseParityValidator::is_valid(&instance),
        AdditionalPropsFalseParityValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{}},"patternProperties":{"^x_":{}},"additionalProperties":false}"#
)]
struct AdditionalPropsPatternParityValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","patternProperties":{"^x_":{}},"additionalProperties":false}"#
)]
struct AdditionalPropsPatternOnlyParityValidator;

#[test]
fn test_additional_properties_pattern_validate_parity() {
    // properties + patternProperties + additionalProperties:false
    let schema = serde_json::json!({"type":"object","properties":{"a":{}},"patternProperties":{"^x_":{}},"additionalProperties":false});
    let instance = serde_json::json!({"a":1,"x_ok":2,"bad":3});
    assert_validate_parity_for(
        &schema,
        AdditionalPropsPatternParityValidator::is_valid(&instance),
        AdditionalPropsPatternParityValidator::validate(&instance),
        &instance,
    );

    // patternProperties only + additionalProperties:false (no `properties`)
    let schema = serde_json::json!({"type":"object","patternProperties":{"^x_":{}},"additionalProperties":false});
    let instance = serde_json::json!({"x_ok":2,"bad":3});
    assert_validate_parity_for(
        &schema,
        AdditionalPropsPatternOnlyParityValidator::is_valid(&instance),
        AdditionalPropsPatternOnlyParityValidator::validate(&instance),
        &instance,
    );
}

// properties with a non-trivial value schema + additionalProperties:false (no patternProperties):
// the value error and the additionalProperties error must interleave in a single instance-order
// pass, so whichever offending key appears first is reported first.
#[jsonschema::validator(
    schema = r#"{"properties":{"m":{"type":"integer"}},"additionalProperties":false}"#
)]
struct AdditionalPropsFalseNonTrivialParityValidator;

#[test_case(serde_json::json!({"m":"x","z":1}) ; "covered_value_before_additional")]
#[test_case(serde_json::json!({"a":1,"m":"x"}) ; "additional_before_covered_value")]
#[test_case(serde_json::json!({"m":"x"}) ; "only_value_error")]
#[test_case(serde_json::json!({"z":1}) ; "only_additional_error")]
fn test_additional_properties_false_nontrivial_validate_parity(instance: serde_json::Value) {
    let schema =
        serde_json::json!({"properties":{"m":{"type":"integer"}},"additionalProperties":false});
    assert_validate_parity_for(
        &schema,
        AdditionalPropsFalseNonTrivialParityValidator::is_valid(&instance),
        AdditionalPropsFalseNonTrivialParityValidator::validate(&instance),
        &instance,
    );
}

// The additionalProperties error precedes a missing-required error.
#[jsonschema::validator(
    schema = r#"{"properties":{"m":{"type":"integer"}},"required":["m"],"additionalProperties":false}"#
)]
struct AdditionalPropsFalseRequiredParityValidator;

#[test_case(serde_json::json!({"m":"x","z":1}) ; "value_error_first")]
#[test_case(serde_json::json!({"z":1}) ; "additional_before_missing_required")]
#[test_case(serde_json::json!({}) ; "missing_required_only")]
fn test_additional_properties_false_required_validate_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"properties":{"m":{"type":"integer"}},"required":["m"],"additionalProperties":false});
    assert_validate_parity_for(
        &schema,
        AdditionalPropsFalseRequiredParityValidator::is_valid(&instance),
        AdditionalPropsFalseRequiredParityValidator::validate(&instance),
        &instance,
    );
}

// Outside the additionalProperties:false single-required fusion, `required` is validated before
// property values, so a missing required field is reported ahead of a covered-value error.
#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]}"#
)]
struct RequiredBeforePropertiesParityValidator;

#[test_case(serde_json::json!({"a":"x"}) ; "missing_required_before_bad_value")]
#[test_case(serde_json::json!({"a":"x","b":2}) ; "bad_value_when_required_met")]
#[test_case(serde_json::json!({}) ; "first_missing_required")]
#[test_case(serde_json::json!({"a":1}) ; "second_missing_required")]
fn test_required_before_properties_validate_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]});
    assert_validate_parity_for(
        &schema,
        RequiredBeforePropertiesParityValidator::is_valid(&instance),
        RequiredBeforePropertiesParityValidator::validate(&instance),
        &instance,
    );
}

// patternProperties value errors and additionalProperties errors interleave in instance order for
// additionalProperties:false; property values come before pattern values otherwise.
#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}},"additionalProperties":false}"#
)]
struct PatternsApFalseParityValidator;

#[test_case(serde_json::json!({"!":1,"x_y":true}) ; "uncovered_before_bad_pattern")]
#[test_case(serde_json::json!({"x_y":true,"z":1}) ; "bad_pattern_before_uncovered")]
#[test_case(serde_json::json!({"a":"s","x_y":true}) ; "bad_property_before_bad_pattern")]
fn test_patterns_ap_false_validate_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}},"additionalProperties":false});
    assert_validate_parity_for(
        &schema,
        PatternsApFalseParityValidator::is_valid(&instance),
        PatternsApFalseParityValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}}}"#
)]
struct PatternsApTrueParityValidator;

#[test_case(serde_json::json!({"a":"s","x_y":true}) ; "bad_property_before_bad_pattern")]
fn test_patterns_ap_true_validate_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}}});
    assert_validate_parity_for(
        &schema,
        PatternsApTrueParityValidator::is_valid(&instance),
        PatternsApTrueParityValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}},"additionalProperties":{"type":"boolean"}}"#
)]
struct PatternsApSchemaParityValidator;

#[test_case(serde_json::json!({"a":"s","x_y":true,"z":1}) ; "property_pattern_additional")]
#[test_case(serde_json::json!({"x_y":true,"z":1}) ; "pattern_before_additional")]
fn test_patterns_ap_schema_validate_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}},"additionalProperties":{"type":"boolean"}});
    assert_validate_parity_for(
        &schema,
        PatternsApSchemaParityValidator::is_valid(&instance),
        PatternsApSchemaParityValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    path = "../benchmark/data/recursive_schema.json",
    draft = referencing::Draft::Draft7
)]
struct InlineValidator;

#[test]
fn test_inline_validator_accepts_benchmark_instance() {
    let instance: serde_json::Value =
        serde_json::from_str(include_str!("../../benchmark/data/recursive_instance.json"))
            .expect("valid JSON");
    assert!(InlineValidator::is_valid(&instance));
    assert!(!InlineValidator::is_valid(&serde_json::json!(null)));
}

#[jsonschema::validator(
    path = "../benchmark/data/openapi.json",
    draft = referencing::Draft::Draft4
)]
struct OpenApiValidator;

#[test]
fn test_openapi_validator_accepts_minimal_valid_document() {
    let valid = serde_json::json!({"openapi": "3.0.0", "info": {"title": "Test", "version": "0"}, "paths": {}});
    assert!(OpenApiValidator::is_valid(&valid));
    assert!(!OpenApiValidator::is_valid(&serde_json::json!({})));
}

#[jsonschema::validator(
    schema = r#"{"$ref": "json-schema:///address"}"#,
    resources = {
        "json-schema:///address" => { schema = r#"{"type": "object", "required": ["street"]}"# },
    }
)]
struct AddressValidator;

#[jsonschema::validator(schema = r#"{"type":["string","integer"],"minLength":2}"#)]
struct MixedTypeWithStringKeywordValidator;

#[jsonschema::validator(schema = r#"{"type":"integer","enum":[1.5]}"#)]
struct IntegerTypeWithNumberEnumValidator;

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":"a"}}},{"type":"object","required":["k"],"properties":{"k":{"const":true}}},{"type":"object","required":["k"],"properties":{"k":{"const":"c"}}}]}"#
)]
struct OneOfMixedConstKindsValidator;

#[jsonschema::validator(schema = r#"{"type":"string","pattern":"^(a|b)$"}"#)]
struct StringAlternationPatternValidator;

#[jsonschema::validator(schema = r#"{"type":"string","pattern":"^(?!eo:)"}"#)]
struct LookaroundPatternValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","pattern":"^ab+$"}"#,
    pattern_options = {
        engine = regex
    }
)]
struct RegexEnginePatternValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","pattern":"^ab+$"}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1_000_000,
        size_limit = 1_000_000,
        dfa_size_limit = 2_000_000,
    }
)]
struct FancyRegexPatternOptionsValidator;

#[jsonschema::validator(schema = r#"{"type":"number","minimum":9007199254740993}"#)]
struct NumericMinimumMixedRepresentationValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","minimum":18446744073709551616}"#)]
struct ArbitraryPrecisionMinimumValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","maximum":18446744073709551616}"#)]
struct ArbitraryPrecisionMaximumValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","exclusiveMinimum":0.1}"#)]
struct ArbitraryPrecisionExclusiveMinimumValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","exclusiveMaximum":0.1}"#)]
struct ArbitraryPrecisionExclusiveMaximumValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","multipleOf":18446744073709551616}"#)]
struct ArbitraryPrecisionMultipleOfBigIntValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","multipleOf":0.1}"#)]
struct ArbitraryPrecisionMultipleOfBigFracValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","multipleOf":1e400}"#)]
struct ArbitraryPrecisionMultipleOfExtremeValidator;

// Strict `exclusive*` comparisons against a bound larger than u64 (the inclusive
// `minimum`/`maximum` validators above cover the non-strict comparisons).
#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","exclusiveMinimum":18446744073709551616}"#)]
struct ArbitraryPrecisionExclusiveMinimumBigIntValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","exclusiveMaximum":18446744073709551616}"#)]
struct ArbitraryPrecisionExclusiveMaximumBigIntValidator;

// Non-strict comparisons against a fractional bound smaller than any integer.
#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","minimum":0.1}"#)]
struct ArbitraryPrecisionMinimumBigFracValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","maximum":0.1}"#)]
struct ArbitraryPrecisionMaximumBigFracValidator;

// Exponents beyond the range f64 can represent: the generated validator must
// still agree with the runtime validator on such extreme schemas.
#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","exclusiveMinimum":1e-2000000}"#)]
struct ArbitraryPrecisionTinyExclusiveMinimumValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","exclusiveMaximum":1e2000000}"#)]
struct ArbitraryPrecisionHugeExclusiveMaximumValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","multipleOf":1e2000000}"#)]
struct ArbitraryPrecisionHugeMultipleOfValidator;

// Inclusive bounds whose exponent overflows f64: the non-strict operators must
// still agree with the runtime validator.
#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","minimum":1e2000000}"#)]
struct ArbitraryPrecisionHugeMinimumValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","maximum":1e2000000}"#)]
struct ArbitraryPrecisionHugeMaximumValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"number","minimum":-1e2000000}"#)]
struct ArbitraryPrecisionHugeNegativeMinimumValidator;

#[jsonschema::validator(schema = r##"{"$ref":"#"}"##)]
struct SelfRefValidator;

#[jsonschema::validator(
    schema = r#"{"anyOf":[{"$ref":"json-schema:///ext#/$defs/A"},{"$ref":"json-schema:///ext#/$defs/B"}]}"#,
    resources = {
        "json-schema:///ext" => { schema = r#"{"$defs":{"A":{"type":"string"},"B":{"type":"integer"}}}"# },
    }
)]
struct ExternalRefFragmentsValidator;

#[jsonschema::validator(schema = r#"{"const":1}"#, draft = referencing::Draft::Draft4)]
struct Draft4ConstIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"properties":{"a":{}},"required":["a"],"additionalProperties":false}"#
)]
struct RequiredInPropertiesValidator;

#[jsonschema::validator(schema = r#"{"type":"number","multipleOf":0.1}"#)]
struct DecimalMultipleOfValidator;

#[jsonschema::validator(schema = r#"{"type":"number","exclusiveMinimum":3}"#)]
struct IntegerExclusiveMinimumValidator;

#[jsonschema::validator(
    schema = r##"{"$defs":{"node":{"type":"array","items":{"$ref":"#/$defs/node"}}},"$ref":"#/$defs/node"}"##
)]
struct RecursiveNodeValidator;

#[jsonschema::validator(schema = r#"{"type":"string","minLength":2,"maxLength":5}"#)]
struct StringMinMaxLengthValidator;

#[jsonschema::validator(schema = r#"{"type":"string","minLength":0,"maxLength":0}"#)]
struct StringEmptyOnlyValidator;

#[jsonschema::validator(schema = r#"{"type":"number","minimum":5,"maximum":5}"#)]
struct NumericEqualBoundsValidator;

#[jsonschema::validator(schema = r#"{"type":"number","minimum":-3,"maximum":-3}"#)]
struct NumericEqualNegativeBoundsValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","additionalProperties":false,"patternProperties":{"^[a-z]{2,}$":{"type":"string"}},"propertyNames":{"pattern":"^[a-z]{2,}$"}}"#
)]
struct PropertyNamesCoveredByPatternValidator;

#[jsonschema::validator(schema = r#"{"type":"object","additionalProperties":false}"#)]
struct AdditionalPropertiesFalseValidator;

// `unevaluatedProperties` paired with each sibling applicator.
#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}},"additionalProperties":{"type":"boolean"},"unevaluatedProperties":false}"#
)]
struct UnevalPropsSiblingsValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","allOf":[{"properties":{"a":{}}}],"unevaluatedProperties":false}"#
)]
struct UnevalPropsAllOfValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","anyOf":[{"properties":{"a":{"type":"integer"}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}],"unevaluatedProperties":false}"#
)]
struct UnevalPropsAnyOfValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","oneOf":[{"properties":{"a":{}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}],"unevaluatedProperties":false}"#
)]
struct UnevalPropsOneOfValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","if":{"properties":{"kind":{"const":"x"}},"required":["kind"]},"then":{"properties":{"x_val":{}}},"else":{"properties":{"y_val":{}}},"unevaluatedProperties":false}"#
)]
struct UnevalPropsIfThenElseValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","dependentSchemas":{"a":{"properties":{"b":{}}}},"unevaluatedProperties":false}"#
)]
struct UnevalPropsDependentSchemasValidator;

#[jsonschema::validator(
    schema = r##"{"type":"object","$ref":"#/$defs/base","$defs":{"base":{"properties":{"a":{}}}},"unevaluatedProperties":false}"##
)]
struct UnevalPropsRefValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","anyOf":[{"allOf":[{"properties":{"a":{}}}],"not":{"required":["z"]}}],"unevaluatedProperties":false}"#
)]
struct UnevalPropsNestedGuardValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{}},"unevaluatedProperties":{"type":"integer"}}"#
)]
struct UnevalPropsSchemaValidator;

// `unevaluatedItems` paired with each sibling applicator.
#[jsonschema::validator(
    schema = r#"{"type":"array","prefixItems":[{"type":"integer"}],"contains":{"type":"string"},"unevaluatedItems":false}"#
)]
struct UnevalItemsPrefixContainsValidator;

#[jsonschema::validator(
    schema = r#"{"type":"array","allOf":[{"prefixItems":[{"type":"integer"}]}],"unevaluatedItems":false}"#
)]
struct UnevalItemsAllOfValidator;

#[jsonschema::validator(
    schema = r#"{"type":"array","anyOf":[{"prefixItems":[{}]},{"prefixItems":[{},{}]}],"unevaluatedItems":false}"#
)]
struct UnevalItemsAnyOfValidator;

#[jsonschema::validator(
    schema = r#"{"type":"array","oneOf":[{"prefixItems":[{"const":1}]},{"prefixItems":[{"const":2},{}]}],"unevaluatedItems":false}"#
)]
struct UnevalItemsOneOfValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"array","items":[{"type":"integer"}],"unevaluatedItems":false}"#
)]
struct UnevalItemsTupleValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"array","items":[{"type":"integer"}],"additionalItems":{"type":"string"},"unevaluatedItems":false}"#
)]
struct UnevalItemsTupleAdditionalValidator;

#[jsonschema::validator(
    schema = r#"{"type":"array","prefixItems":[{}],"unevaluatedItems":{"type":"integer"}}"#
)]
struct UnevalItemsSchemaValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"type":"object","properties":{"child":{"$recursiveRef":"#"}},"unevaluatedProperties":false}"##
)]
struct RecursiveUnevaluatedPropertiesValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"items":[{"type":"integer"}],"unevaluatedItems":{"$recursiveRef":"#"}}"##
)]
struct RecursiveUnevaluatedItemsValidator;

// `$recursiveRef` whose target lacks `$recursiveAnchor` behaves like a plain `$ref`.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"object","properties":{"a":{"type":"integer"}},"$recursiveRef":"#","unevaluatedProperties":false}"##
)]
struct NonAnchorRecursiveRefUnevalPropsValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","items":[{"type":"integer"}],"$recursiveRef":"#","unevaluatedItems":false}"##
)]
struct NonAnchorRecursiveRefUnevalItemsValidator;

// `$dynamicRef` to a static `$anchor` (no matching `$dynamicAnchor`) resolves lexically.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","$dynamicRef":"#t","$defs":{"t":{"$anchor":"t","properties":{"b":{"type":"integer"}}}},"unevaluatedProperties":false}"##
)]
struct StaticDynamicRefUnevalPropsValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicRef":"#t","$defs":{"t":{"$anchor":"t","prefixItems":[{"type":"integer"}]}},"unevaluatedItems":false}"##
)]
struct StaticDynamicRefUnevalItemsValidator;

// A lexical self `$dynamicRef` (no matching `$dynamicAnchor`) as a direct sibling of the unevaluated keyword.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicRef":"#","type":"object","properties":{"a":{"type":"integer"}},"unevaluatedProperties":false}"##
)]
struct DynamicRefUnevalPropsValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicRef":"#","prefixItems":[{"type":"integer"}],"unevaluatedItems":false}"##
)]
struct DynamicRefUnevalItemsValidator;

// A self-referential `$dynamicRef`/`$dynamicAnchor` whose evaluated-key/item walk cycles back to the same schema on the same instance.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"node","type":"object","$dynamicRef":"#node","unevaluatedProperties":false}"##
)]
struct DynamicAnchorCycleUnevalPropsValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"node","type":"array","$dynamicRef":"#node","unevaluatedItems":false}"##
)]
struct DynamicAnchorCycleUnevalItemsValidator;

// `oneOf` whose branches are all boolean schemas.
#[jsonschema::validator(
    schema = r#"{"type":"object","oneOf":[true,false],"unevaluatedProperties":false}"#
)]
struct BooleanOneOfUnevalPropsValidator;

#[jsonschema::validator(
    schema = r#"{"$ref":"json-schema:///num","type":"string"}"#,
    draft = referencing::Draft::Draft201909,
    resources = {
        "json-schema:///num" => { schema = r#"{"type":"number"}"# },
    }
)]
struct RefSiblingDraft201909Validator;

#[jsonschema::validator(
    schema = r#"{"$ref":"json-schema:///num","type":"string"}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///num" => { schema = r#"{"type":"number"}"# },
    }
)]
struct RefSiblingDraft202012Validator;

#[jsonschema::validator(
    schema = r#"{"dependentSchemas":{"foo":{"required":["bar"]}}}"#,
    draft = referencing::Draft::Draft201909
)]
struct DependentSchemasDraft201909Validator;

#[jsonschema::validator(
    schema = r#"{"type":"array","contains":{"type":"integer"},"minContains":2,"maxContains":3}"#,
    draft = referencing::Draft::Draft201909
)]
struct ContainsBoundsDraft201909Validator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"ipv4"}"#,
    draft = referencing::Draft::Draft201909
)]
struct Draft201909FormatDefaultOffValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"ipv4"}"#,
    draft = referencing::Draft::Draft201909,
    validate_formats = true
)]
struct Draft201909FormatEnabledValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"email"}"#,
    draft = referencing::Draft::Draft7,
    validate_formats = true,
    email_options = {
        required_tld = true,
        allow_domain_literal = false,
        allow_display_text = false
    }
)]
struct EmailOptionsConfiguredValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"email"}"#,
    draft = referencing::Draft::Draft7,
    validate_formats = true,
    email_options = {
        minimum_sub_domains = 2,
        allow_domain_literal = true,
        allow_display_text = true,
    }
)]
struct EmailOptionsMinimumSubDomainsValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"email"}"#,
    draft = referencing::Draft::Draft7,
    validate_formats = true,
    email_options = {
        no_minimum_sub_domains = true,
        allow_domain_literal = false,
        allow_display_text = false,
    }
)]
struct EmailOptionsNoMinimumSubDomainsValidator;

#[jsonschema::validator(
    schema = r#"{"$ref":"defs.json#/$defs/item"}"#,
    base_uri = "json-schema:///root/main.json",
    resources = {
        "json-schema:///root/defs.json" => { schema = r#"{"$defs":{"item":{"type":"integer","minimum":1}}}"# },
    }
)]
struct BaseUriRelativeRefValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"currency"}"#,
    draft = referencing::Draft::Draft7,
    formats = {
        "currency" => crate::is_currency_format,
    }
)]
struct CustomFormatDraft7Validator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"currency"}"#,
    draft = referencing::Draft::Draft201909,
    formats = {
        "currency" => crate::is_currency_format,
    }
)]
struct CustomFormatDraft201909DefaultOffValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"currency"}"#,
    draft = referencing::Draft::Draft201909,
    validate_formats = true,
    formats = {
        "currency" => crate::is_currency_format,
    }
)]
struct CustomFormatDraft201909EnabledValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"email"}"#,
    draft = referencing::Draft::Draft7,
    validate_formats = true,
    formats = {
        "email" => crate::is_literal_x,
    }
)]
struct CustomFormatOverrideBuiltInValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"made-up"}"#,
    draft = referencing::Draft::Draft7,
    validate_formats = true,
    ignore_unknown_formats = true
)]
struct UnknownFormatIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","minLength":1,"format":"made-up"}"#,
    draft = referencing::Draft::Draft7,
    validate_formats = true,
    ignore_unknown_formats = true
)]
struct UnknownFormatIgnoredWithSiblingValidator;

#[jsonschema::validator(
    schema = r#"{"type":"number","format":"made-up"}"#,
    draft = referencing::Draft::Draft201909
)]
struct NumberTypeUnknownFormatDraft201909Validator;

#[jsonschema::validator(
    schema = r#"{"type":"number","format":"made-up"}"#,
    draft = referencing::Draft::Draft7,
    validate_formats = true,
    ignore_unknown_formats = true
)]
struct NumberTypeUnknownFormatDraft7Validator;

#[jsonschema::validator(
    schema = r#"{"dependentRequired":{"foo":["bar"]}}"#,
    draft = referencing::Draft::Draft7
)]
struct Draft7DependentRequiredIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"if":{"const":1},"then":false}"#,
    draft = referencing::Draft::Draft4
)]
struct Draft4IfThenIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"if":{"const":1},"then":false}"#,
    draft = referencing::Draft::Draft6
)]
struct Draft6IfThenIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","dependentSchemas":{"foo":{"required":["bar"]}}}"#,
    draft = referencing::Draft::Draft7
)]
struct Draft7DependentSchemasIgnoredWithTypeValidator;

#[jsonschema::validator(
    schema = r#"{"propertyNames":{"pattern":"^x"}}"#,
    draft = referencing::Draft::Draft4
)]
struct Draft4PropertyNamesIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"type":"string","contentEncoding":"base64"}"#,
    draft = referencing::Draft::Draft4
)]
struct Draft4ContentEncodingIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"contains":{"type":"integer"}}"#,
    draft = referencing::Draft::Draft4
)]
struct Draft4ContainsIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"type":"array","contains":{"type":"integer"},"minContains":2}"#,
    draft = referencing::Draft::Draft7
)]
struct Draft7MinContainsIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"$ref":"json-schema:///future","type":"array"}"#,
    draft = referencing::Draft::Draft201909,
    resources = {
        "json-schema:///future" => { schema = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]}"# },
    }
)]
struct Draft201909RefTo202012PrefixItemsValidator;

#[jsonschema::validator(
    schema = r##"{"$recursiveAnchor":true,"type":"object","properties":{"child":{"$recursiveRef":"#"}}}"##,
    draft = referencing::Draft::Draft201909
)]
struct RecursiveRefDraft201909Validator;

#[jsonschema::validator(schema = r#"{"type":"integer","minimum":0}"#)]
struct IntegerMinimumValidator;

#[jsonschema::validator(schema = r#"{"type":["integer","string"],"maximum":0}"#)]
struct IntegerOrStringMaximumValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"foo":true},"dependencies":{"foo":{"properties":{"bar":true}}},"unevaluatedProperties":false}"#,
    draft = referencing::Draft::Draft201909
)]
struct DependenciesUnevaluatedPropertiesValidator;

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"required":["kind"],"properties":{"kind":{"const":"a"}}},{"required":["kind"],"properties":{"kind":{"const":"b"}}},{"type":"string"}]}"#
)]
struct OneOfDiscriminatorWithScalarBranchValidator;

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["tag"],"properties":{"tag":{"const":1},"a":{"type":"string"}}},{"type":"object","required":["tag"],"properties":{"tag":{"const":2},"a":{"type":"integer"}}}]}"#
)]
struct OneOfIntDiscriminatorValidator;

#[jsonschema::validator(
    schema = r#"{"properties":{"x":{"$id":"http://example.com/nested/anchor.json","properties":{"in":{"$ref":"item.json"}}}}}"#,
    draft = referencing::Draft::Draft4,
    base_uri = "json-schema:///root/main.json",
    resources = {
        "http://example.com/nested/anchor.json" => { schema = r#"{"type":"object"}"# },
        "json-schema:///root/item.json" => { schema = r#"{"type":"string"}"# },
        "http://example.com/nested/item.json" => { schema = r#"{"type":"integer"}"# },
    }
)]
struct Draft4ModernIdIgnoredValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-validation","type":"string","minLength":2,"enum":["ab"]}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct NoValidationVocabularyValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-applicator","type":"object","properties":{"a":{"type":"string"}},"additionalProperties":false,"allOf":[{"required":["a"]}]}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/no-applicator" => { schema = r#"{"$id":"json-schema:///meta/no-applicator","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":false,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct NoApplicatorVocabularyValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-unevaluated","type":"object","properties":{"a":{"type":"string"}},"unevaluatedProperties":false}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/no-unevaluated" => { schema = r#"{"$id":"json-schema:///meta/no-unevaluated","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":false,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct NoUnevaluatedVocabularyValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-applicator","type":"array","items":{"type":"string"},"contains":{"type":"integer"}}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/no-applicator" => { schema = r#"{"$id":"json-schema:///meta/no-applicator","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":false,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct NoApplicatorArrayVocabularyValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-unevaluated","type":"array","prefixItems":[{"type":"string"}],"unevaluatedItems":false}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/no-unevaluated" => { schema = r#"{"$id":"json-schema:///meta/no-unevaluated","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":false,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct NoUnevaluatedArrayVocabularyValidator;

#[test]
fn test_external_resources() {
    assert!(AddressValidator::is_valid(
        &serde_json::json!({"street": "Main St"})
    ));
    assert!(!AddressValidator::is_valid(&serde_json::json!({})));
}

// `is_valid()` and `validate()` must agree, including for an unconstrained-but-allowed type.
#[test_case(serde_json::json!("ab"), true ; "string_meets_min_length")]
#[test_case(serde_json::json!(42), true ; "integer_without_number_keywords")]
#[test_case(serde_json::json!("a"), false ; "string_too_short")]
#[test_case(serde_json::json!(1.5), false ; "non_integer_number")]
fn test_typed_fallback_is_valid_and_validate_agree(instance: serde_json::Value, valid: bool) {
    assert_eq!(
        MixedTypeWithStringKeywordValidator::is_valid(&instance),
        valid
    );
    assert_eq!(
        MixedTypeWithStringKeywordValidator::validate(&instance).is_ok(),
        valid
    );
}

// A required property whose `const` kinds disagree across `oneOf` branches.
#[test_case(serde_json::json!({"k": "a"}), true ; "string_branch_a")]
#[test_case(serde_json::json!({"k": true}), true ; "bool_branch")]
#[test_case(serde_json::json!({"k": "c"}), true ; "string_branch_c")]
#[test_case(serde_json::json!({"k": "x"}), false ; "unmatched_string")]
#[test_case(serde_json::json!({"k": false}), false ; "unmatched_bool")]
fn test_one_of_mixed_const_kinds_discriminator(instance: serde_json::Value, valid: bool) {
    assert_eq!(OneOfMixedConstKindsValidator::is_valid(&instance), valid);
}

#[test]
fn test_integer_type_is_not_redundant_with_number_enum() {
    assert!(!IntegerTypeWithNumberEnumValidator::is_valid(
        &serde_json::json!(1.5)
    ));
}

#[test]
fn test_string_pattern_alternation_validator_compiles_and_validates() {
    assert!(StringAlternationPatternValidator::is_valid(
        &serde_json::json!("a")
    ));
    assert!(StringAlternationPatternValidator::is_valid(
        &serde_json::json!("b")
    ));
    assert!(!StringAlternationPatternValidator::is_valid(
        &serde_json::json!("c")
    ));
}

#[test_case(serde_json::json!("proj:epsg"), true ; "lookaround_accepts_non_eo")]
#[test_case(serde_json::json!("eo:bands"), false ; "lookaround_rejects_eo_prefix")]
fn test_valid_ecma_pattern_does_not_panic(instance: serde_json::Value, expected: bool) {
    assert_eq!(LookaroundPatternValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!("ab"), true ; "regex_engine_accepts_matching")]
#[test_case(serde_json::json!("abbb"), true ; "regex_engine_accepts_longer_match")]
#[test_case(serde_json::json!("a"), false ; "regex_engine_rejects_non_match")]
fn test_pattern_options_regex_engine(instance: serde_json::Value, expected: bool) {
    assert_eq!(RegexEnginePatternValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!("a"), false ; "below_minimum")]
#[test_case(serde_json::json!("abc"), true ; "within_range")]
#[test_case(serde_json::json!("abcdef"), false ; "above_maximum")]
fn test_string_min_and_max_length_match_runtime(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({"type":"string","minLength":2,"maxLength":5});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(StringMinMaxLengthValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!(""), true ; "empty_allowed")]
#[test_case(serde_json::json!("a"), false ; "non_empty_rejected")]
fn test_string_empty_only_match_runtime(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({"type":"string","minLength":0,"maxLength":0});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(StringEmptyOnlyValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!(4) ; "below_reports_minimum")]
#[test_case(serde_json::json!(5) ; "equal_is_valid")]
#[test_case(serde_json::json!(6) ; "above_reports_maximum")]
#[test_case(serde_json::json!(5.5) ; "float_instance_skipped")]
#[test_case(serde_json::json!("x") ; "non_number_skipped")]
fn test_numeric_equal_bounds_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"number","minimum":5,"maximum":5});
    assert_validate_parity_for(
        &schema,
        NumericEqualBoundsValidator::is_valid(&instance),
        NumericEqualBoundsValidator::validate(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!(-3) ; "equal_is_valid")]
#[test_case(serde_json::json!(-4) ; "below_reports_minimum")]
#[test_case(serde_json::json!(-3.5) ; "fractional_below")]
fn test_numeric_equal_negative_bounds_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"number","minimum":-3,"maximum":-3});
    assert_validate_parity_for(
        &schema,
        NumericEqualNegativeBoundsValidator::is_valid(&instance),
        NumericEqualNegativeBoundsValidator::validate(&instance),
        &instance,
    );
}

#[cfg(feature = "arbitrary-precision")]
#[test_case(false ; "huge_negative_reports_minimum")]
#[test_case(true ; "huge_positive_reports_maximum")]
fn test_numeric_equal_bounds_arbitrary_precision(positive: bool) {
    let sign = if positive { "" } else { "-" };
    let raw = format!("{sign}{}.5", "9".repeat(320));
    let instance: serde_json::Value = serde_json::from_str(&raw).expect("valid number");
    let schema = serde_json::json!({"type":"number","minimum":5,"maximum":5});
    assert_validate_parity_for(
        &schema,
        NumericEqualBoundsValidator::is_valid(&instance),
        NumericEqualBoundsValidator::validate(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!({"abc": "x"}) ; "all_keys_covered_valid")]
#[test_case(serde_json::json!({"AB": "x"}) ; "key_fails_pattern_reports_property_names")]
#[test_case(serde_json::json!({"abc": 5}) ; "covered_key_wrong_type")]
#[test_case(serde_json::json!({"abc": "x", "9": 1}) ; "mixed_name_failure_first")]
fn test_property_names_covered_by_pattern_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","additionalProperties":false,"patternProperties":{"^[a-z]{2,}$":{"type":"string"}},"propertyNames":{"pattern":"^[a-z]{2,}$"}});
    assert_validate_parity_for(
        &schema,
        PropertyNamesCoveredByPatternValidator::is_valid(&instance),
        PropertyNamesCoveredByPatternValidator::validate(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!({}), true ; "empty_allowed")]
#[test_case(serde_json::json!({"a": 1}), false ; "any_property_rejected")]
#[test_case(serde_json::json!("not an object"), false ; "non_object_rejected_by_type")]
fn test_additional_properties_false_match_runtime(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({"type":"object","additionalProperties":false});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        AdditionalPropertiesFalseValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!({}) ; "empty_object")]
#[test_case(serde_json::json!({"child": {}}) ; "evaluated_child")]
#[test_case(serde_json::json!({"other": 1}) ; "unevaluated_key")]
#[test_case(serde_json::json!({"child": {"other": 1}}) ; "nested_unevaluated_key")]
fn test_recursive_unevaluated_properties_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"type":"object","properties":{"child":{"$recursiveRef":"#"}},"unevaluatedProperties":false});
    assert_is_valid_parity(
        &schema,
        RecursiveUnevaluatedPropertiesValidator::is_valid(&instance),
        &instance,
    );
}

#[test]
fn test_recursive_unevaluated_properties_validate_parity() {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"type":"object","properties":{"child":{"$recursiveRef":"#"}},"unevaluatedProperties":false});
    let instance = serde_json::json!({"other": 1});
    assert_validate_parity_for(
        &schema,
        RecursiveUnevaluatedPropertiesValidator::is_valid(&instance),
        RecursiveUnevaluatedPropertiesValidator::validate(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!([]) ; "empty_array")]
#[test_case(serde_json::json!([5]) ; "only_prefix_item")]
#[test_case(serde_json::json!([5, [3]]) ; "recursive_unevaluated_item")]
#[test_case(serde_json::json!([5, "x"]) ; "unevaluated_non_array")]
fn test_recursive_unevaluated_items_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"items":[{"type":"integer"}],"unevaluatedItems":{"$recursiveRef":"#"}});
    assert_is_valid_parity(
        &schema,
        RecursiveUnevaluatedItemsValidator::is_valid(&instance),
        &instance,
    );
}

#[test]
fn test_recursive_unevaluated_items_validate_parity() {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"items":[{"type":"integer"}],"unevaluatedItems":{"$recursiveRef":"#"}});
    let instance = serde_json::json!([5, ["x"]]);
    assert_validate_parity_for(
        &schema,
        RecursiveUnevaluatedItemsValidator::is_valid(&instance),
        RecursiveUnevaluatedItemsValidator::validate(&instance),
        &instance,
    );
    assert!(RecursiveUnevaluatedItemsValidator::validate(&instance).is_err());
}

#[test_case(serde_json::json!({"a": 1}) ; "evaluated_property")]
#[test_case(serde_json::json!({"a": 1, "other": 2}) ; "unevaluated_property")]
#[test_case(serde_json::json!({}) ; "empty")]
fn test_non_anchor_recursive_ref_uneval_props(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2019-09/schema","type":"object","properties":{"a":{"type":"integer"}},"$recursiveRef":"#","unevaluatedProperties":false});
    assert_is_valid_parity(
        &schema,
        NonAnchorRecursiveRefUnevalPropsValidator::is_valid(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!([1]) ; "evaluated_item")]
#[test_case(serde_json::json!([1, 2]) ; "unevaluated_item")]
#[test_case(serde_json::json!([]) ; "empty")]
fn test_non_anchor_recursive_ref_uneval_items(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2019-09/schema","items":[{"type":"integer"}],"$recursiveRef":"#","unevaluatedItems":false});
    assert_is_valid_parity(
        &schema,
        NonAnchorRecursiveRefUnevalItemsValidator::is_valid(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!({"b": 1}) ; "evaluated_property")]
#[test_case(serde_json::json!({"b": 1, "other": 2}) ; "unevaluated_property")]
#[test_case(serde_json::json!({}) ; "empty")]
fn test_static_dynamic_ref_uneval_props(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","$dynamicRef":"#t","$defs":{"t":{"$anchor":"t","properties":{"b":{"type":"integer"}}}},"unevaluatedProperties":false});
    assert_is_valid_parity(
        &schema,
        StaticDynamicRefUnevalPropsValidator::is_valid(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!([1]) ; "evaluated_item")]
#[test_case(serde_json::json!([1, 2]) ; "unevaluated_item")]
#[test_case(serde_json::json!([]) ; "empty")]
fn test_static_dynamic_ref_uneval_items(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicRef":"#t","$defs":{"t":{"$anchor":"t","prefixItems":[{"type":"integer"}]}},"unevaluatedItems":false});
    assert_is_valid_parity(
        &schema,
        StaticDynamicRefUnevalItemsValidator::is_valid(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!({"a": 1}) ; "evaluated_property")]
#[test_case(serde_json::json!({"a": 1, "other": 2}) ; "unevaluated_property")]
#[test_case(serde_json::json!({}) ; "empty")]
fn test_dynamic_ref_uneval_props(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicRef":"#","type":"object","properties":{"a":{"type":"integer"}},"unevaluatedProperties":false});
    assert_is_valid_parity(
        &schema,
        DynamicRefUnevalPropsValidator::is_valid(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!([1]) ; "evaluated_item")]
#[test_case(serde_json::json!([1, 2]) ; "unevaluated_item")]
#[test_case(serde_json::json!([]) ; "empty")]
fn test_dynamic_ref_uneval_items(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicRef":"#","prefixItems":[{"type":"integer"}],"unevaluatedItems":false});
    assert_is_valid_parity(
        &schema,
        DynamicRefUnevalItemsValidator::is_valid(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!({}) ; "empty")]
#[test_case(serde_json::json!({"a": 1}) ; "unevaluated_property")]
fn test_dynamic_anchor_cycle_uneval_props(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"node","type":"object","$dynamicRef":"#node","unevaluatedProperties":false});
    assert_is_valid_parity(
        &schema,
        DynamicAnchorCycleUnevalPropsValidator::is_valid(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!([]) ; "empty")]
#[test_case(serde_json::json!([1]) ; "unevaluated_item")]
fn test_dynamic_anchor_cycle_uneval_items(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"node","type":"array","$dynamicRef":"#node","unevaluatedItems":false});
    assert_is_valid_parity(
        &schema,
        DynamicAnchorCycleUnevalItemsValidator::is_valid(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!({}) ; "empty_matches_true_branch")]
#[test_case(serde_json::json!({"a": 1}) ; "unevaluated_property")]
fn test_boolean_one_of_uneval_props(instance: serde_json::Value) {
    let schema =
        serde_json::json!({"type":"object","oneOf":[true,false],"unevaluatedProperties":false});
    assert_is_valid_parity(
        &schema,
        BooleanOneOfUnevalPropsValidator::is_valid(&instance),
        &instance,
    );
}

macro_rules! uneval_differential {
    ($name:ident, $validator:ty, $schema:literal, [$($inst:expr),+ $(,)?]) => {
        #[test]
        fn $name() {
            let schema: serde_json::Value =
                serde_json::from_str($schema).expect("valid schema json");
            let runtime = jsonschema::validator_for(&schema).expect("valid schema");
            for inst in [$($inst),+] {
                assert_eq!(
                    <$validator>::is_valid(&inst),
                    runtime.is_valid(&inst),
                    "codegen/runtime mismatch for {inst} under {schema}"
                );
            }
        }
    };
}

uneval_differential!(
    test_uneval_props_siblings,
    UnevalPropsSiblingsValidator,
    r#"{"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}},"additionalProperties":{"type":"boolean"},"unevaluatedProperties":false}"#,
    [
        serde_json::json!({"a":1}),
        serde_json::json!({"x_1":"s"}),
        serde_json::json!({"z":true}),
        serde_json::json!({"a":"x"}),
        serde_json::json!({"x_1":5})
    ]
);
uneval_differential!(
    test_uneval_props_all_of,
    UnevalPropsAllOfValidator,
    r#"{"type":"object","allOf":[{"properties":{"a":{}}}],"unevaluatedProperties":false}"#,
    [
        serde_json::json!({"a":1}),
        serde_json::json!({"b":1}),
        serde_json::json!({})
    ]
);
uneval_differential!(
    test_uneval_props_any_of,
    UnevalPropsAnyOfValidator,
    r#"{"type":"object","anyOf":[{"properties":{"a":{"type":"integer"}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}],"unevaluatedProperties":false}"#,
    [
        serde_json::json!({"a":1}),
        serde_json::json!({"b":2}),
        serde_json::json!({"a":1,"c":3}),
        serde_json::json!({})
    ]
);
uneval_differential!(
    test_uneval_props_one_of,
    UnevalPropsOneOfValidator,
    r#"{"type":"object","oneOf":[{"properties":{"a":{}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}],"unevaluatedProperties":false}"#,
    [
        serde_json::json!({"a":1}),
        serde_json::json!({"b":1}),
        serde_json::json!({"a":1,"b":1}),
        serde_json::json!({"a":1,"c":1})
    ]
);
uneval_differential!(
    test_uneval_props_if_then_else,
    UnevalPropsIfThenElseValidator,
    r#"{"type":"object","if":{"properties":{"kind":{"const":"x"}},"required":["kind"]},"then":{"properties":{"x_val":{}}},"else":{"properties":{"y_val":{}}},"unevaluatedProperties":false}"#,
    [
        serde_json::json!({"kind":"x","x_val":1}),
        serde_json::json!({"y_val":1}),
        serde_json::json!({"kind":"x","y_val":1}),
        serde_json::json!({"z":1})
    ]
);
uneval_differential!(
    test_uneval_props_dependent_schemas,
    UnevalPropsDependentSchemasValidator,
    r#"{"type":"object","dependentSchemas":{"a":{"properties":{"b":{}}}},"unevaluatedProperties":false}"#,
    [
        serde_json::json!({"a":1,"b":2}),
        serde_json::json!({"b":2}),
        serde_json::json!({"c":1}),
        serde_json::json!({})
    ]
);
uneval_differential!(
    test_uneval_props_ref,
    UnevalPropsRefValidator,
    r##"{"type":"object","$ref":"#/$defs/base","$defs":{"base":{"properties":{"a":{}}}},"unevaluatedProperties":false}"##,
    [serde_json::json!({"a":1}), serde_json::json!({"b":1})]
);
uneval_differential!(
    test_uneval_props_nested_guard,
    UnevalPropsNestedGuardValidator,
    r#"{"type":"object","anyOf":[{"allOf":[{"properties":{"a":{}}}],"not":{"required":["z"]}}],"unevaluatedProperties":false}"#,
    [
        serde_json::json!({"a":1}),
        serde_json::json!({"z":1}),
        serde_json::json!({"a":1,"b":2})
    ]
);
uneval_differential!(
    test_uneval_props_schema_form,
    UnevalPropsSchemaValidator,
    r#"{"type":"object","properties":{"a":{}},"unevaluatedProperties":{"type":"integer"}}"#,
    [
        serde_json::json!({"a":1}),
        serde_json::json!({"b":5}),
        serde_json::json!({"b":"s"})
    ]
);
uneval_differential!(
    test_uneval_items_prefix_contains,
    UnevalItemsPrefixContainsValidator,
    r#"{"type":"array","prefixItems":[{"type":"integer"}],"contains":{"type":"string"},"unevaluatedItems":false}"#,
    [
        serde_json::json!([1, "s"]),
        serde_json::json!([1, 2]),
        serde_json::json!([1, "s", 3]),
        serde_json::json!([1])
    ]
);
uneval_differential!(
    test_uneval_items_all_of,
    UnevalItemsAllOfValidator,
    r#"{"type":"array","allOf":[{"prefixItems":[{"type":"integer"}]}],"unevaluatedItems":false}"#,
    [
        serde_json::json!([1]),
        serde_json::json!([1, 2]),
        serde_json::json!([])
    ]
);
uneval_differential!(
    test_uneval_items_any_of,
    UnevalItemsAnyOfValidator,
    r#"{"type":"array","anyOf":[{"prefixItems":[{}]},{"prefixItems":[{},{}]}],"unevaluatedItems":false}"#,
    [
        serde_json::json!([1]),
        serde_json::json!([1, 2]),
        serde_json::json!([1, 2, 3])
    ]
);
uneval_differential!(
    test_uneval_items_one_of,
    UnevalItemsOneOfValidator,
    r#"{"type":"array","oneOf":[{"prefixItems":[{"const":1}]},{"prefixItems":[{"const":2},{}]}],"unevaluatedItems":false}"#,
    [
        serde_json::json!([1]),
        serde_json::json!([2, 3]),
        serde_json::json!([1, 2])
    ]
);
uneval_differential!(
    test_uneval_items_tuple,
    UnevalItemsTupleValidator,
    r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"array","items":[{"type":"integer"}],"unevaluatedItems":false}"#,
    [
        serde_json::json!([1]),
        serde_json::json!([1, 2]),
        serde_json::json!([])
    ]
);
uneval_differential!(
    test_uneval_items_tuple_additional,
    UnevalItemsTupleAdditionalValidator,
    r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"array","items":[{"type":"integer"}],"additionalItems":{"type":"string"},"unevaluatedItems":false}"#,
    [
        serde_json::json!([1]),
        serde_json::json!([1, "s"]),
        serde_json::json!([1, 2])
    ]
);
uneval_differential!(
    test_uneval_items_schema_form,
    UnevalItemsSchemaValidator,
    r#"{"type":"array","prefixItems":[{}],"unevaluatedItems":{"type":"integer"}}"#,
    [
        serde_json::json!([1]),
        serde_json::json!([1, 2]),
        serde_json::json!([1, "s"])
    ]
);

#[test_case(serde_json::json!("ab"), true ; "fancy_engine_accepts_matching")]
#[test_case(serde_json::json!("abbb"), true ; "fancy_engine_accepts_longer_match")]
#[test_case(serde_json::json!("a"), false ; "fancy_engine_rejects_non_match")]
fn test_pattern_options_fancy_engine(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        FancyRegexPatternOptionsValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!(9_007_199_254_740_992.0), false ; "below_minimum_decimal_form")]
#[test_case(serde_json::json!(9_007_199_254_740_994.0), true ; "above_minimum_decimal_form")]
#[test_case(serde_json::json!(9_007_199_254_740_992_u64), false ; "below_minimum_integer_form")]
#[test_case(serde_json::json!(9_007_199_254_740_994_u64), true ; "above_minimum_integer_form")]
fn test_numeric_minimum_mixed_representations_match_runtime(
    instance: serde_json::Value,
    expected: bool,
) {
    let schema = serde_json::json!({"type":"number","minimum":9_007_199_254_740_993_u64});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        NumericMinimumMixedRepresentationValidator::is_valid(&instance),
        expected
    );
}

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"integer"}"#)]
struct ArbitraryPrecisionIntegerTypeValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"integer"}"#, draft = referencing::Draft::Draft4)]
struct ArbitraryPrecisionDraft4IntegerTypeValidator;

// Integer typing for numbers outside the i64/u64/f64 range must match the runtime validator in both draft semantics.
#[cfg(feature = "arbitrary-precision")]
#[test_case("1e400" ; "huge_scientific_integer")]
#[test_case("10000000000000000000000000000000000001" ; "huge_plain_integer")]
#[test_case("1.0" ; "decimal_point_integer_value")]
#[test_case("1e308" ; "in_f64_range_scientific")]
#[test_case("1.5" ; "fractional")]
fn test_arbitrary_precision_integer_type_matches_runtime(instance_json: &str) {
    let instance: serde_json::Value =
        serde_json::from_str(instance_json).expect("valid instance json");
    assert_eq!(
        ArbitraryPrecisionIntegerTypeValidator::is_valid(&instance),
        runtime_valid(&serde_json::json!({"type":"integer"}), &instance),
        "draft 2020-12 divergence for {instance_json}"
    );
    let draft4_runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft4)
        .build(&serde_json::json!({"type":"integer"}))
        .expect("valid schema")
        .is_valid(&instance);
    assert_eq!(
        ArbitraryPrecisionDraft4IntegerTypeValidator::is_valid(&instance),
        draft4_runtime,
        "draft 4 divergence for {instance_json}"
    );
}

#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"type":"number","minimum":18446744073709551616}"#, "18446744073709551615"; "minimum_bigint_below")]
#[test_case(r#"{"type":"number","minimum":18446744073709551616}"#, "1e20"; "minimum_bigint_scientific_above")]
#[test_case(r#"{"type":"number","maximum":18446744073709551616}"#, "18446744073709551617"; "maximum_bigint_above")]
#[test_case(r#"{"type":"number","maximum":18446744073709551616}"#, "1e15"; "maximum_bigint_scientific_below")]
#[test_case(r#"{"type":"number","exclusiveMinimum":0.1}"#, "0.1"; "exclusive_minimum_bigfrac_boundary")]
#[test_case(r#"{"type":"number","exclusiveMinimum":0.1}"#, "0.1000000000000000000001"; "exclusive_minimum_bigfrac_above")]
#[test_case(r#"{"type":"number","exclusiveMaximum":0.1}"#, "0.1"; "exclusive_maximum_bigfrac_boundary")]
#[test_case(r#"{"type":"number","exclusiveMaximum":0.1}"#, "0.0999999999999999999999"; "exclusive_maximum_bigfrac_below")]
#[test_case(r#"{"type":"number","multipleOf":18446744073709551616}"#, "36893488147419103232"; "multiple_of_bigint_valid")]
#[test_case(r#"{"type":"number","multipleOf":18446744073709551616}"#, "18446744073709551617"; "multiple_of_bigint_invalid")]
#[test_case(r#"{"type":"number","multipleOf":0.1}"#, "0.3"; "multiple_of_bigfrac_valid")]
#[test_case(r#"{"type":"number","multipleOf":0.1}"#, "0.35"; "multiple_of_bigfrac_invalid")]
#[test_case(r#"{"type":"number","multipleOf":1e400}"#, "2e400"; "multiple_of_extreme_ignored")]
// A bound larger than u64 compared against instances written as a small integer,
// a negative integer, and a fraction.
#[test_case(r#"{"type":"number","minimum":18446744073709551616}"#, "5"; "minimum_bigint_vs_small_u64")]
#[test_case(r#"{"type":"number","minimum":18446744073709551616}"#, "-5"; "minimum_bigint_vs_negative_i64")]
#[test_case(r#"{"type":"number","minimum":18446744073709551616}"#, "0.5"; "minimum_bigint_vs_fractional_f64")]
#[test_case(r#"{"type":"number","maximum":18446744073709551616}"#, "5"; "maximum_bigint_vs_small_u64")]
#[test_case(r#"{"type":"number","maximum":18446744073709551616}"#, "-5"; "maximum_bigint_vs_negative_i64")]
#[test_case(r#"{"type":"number","maximum":18446744073709551616}"#, "0.5"; "maximum_bigint_vs_fractional_f64")]
// A fractional bound compared against non-fractional instances: a small integer,
// a negative integer, and an integer larger than u64.
#[test_case(r#"{"type":"number","exclusiveMinimum":0.1}"#, "5"; "exclusive_minimum_bigfrac_vs_small_u64")]
#[test_case(r#"{"type":"number","exclusiveMinimum":0.1}"#, "-5"; "exclusive_minimum_bigfrac_vs_negative_i64")]
#[test_case(r#"{"type":"number","exclusiveMinimum":0.1}"#, "1e20"; "exclusive_minimum_bigfrac_vs_big_integer")]
#[test_case(r#"{"type":"number","exclusiveMaximum":0.1}"#, "5"; "exclusive_maximum_bigfrac_vs_small_u64")]
#[test_case(r#"{"type":"number","exclusiveMaximum":0.1}"#, "-5"; "exclusive_maximum_bigfrac_vs_negative_i64")]
#[test_case(r#"{"type":"number","exclusiveMaximum":0.1}"#, "1e20"; "exclusive_maximum_bigfrac_vs_big_integer")]
// A divisor larger than u64 checked against a small integer, a negative integer, and a fraction.
#[test_case(r#"{"type":"number","multipleOf":18446744073709551616}"#, "5"; "multiple_of_bigint_vs_small_u64")]
#[test_case(r#"{"type":"number","multipleOf":18446744073709551616}"#, "-4"; "multiple_of_bigint_vs_negative_i64")]
#[test_case(r#"{"type":"number","multipleOf":18446744073709551616}"#, "0.5"; "multiple_of_bigint_vs_fractional")]
// A fractional divisor checked against an integer larger than u64 and small integers.
#[test_case(r#"{"type":"number","multipleOf":0.1}"#, "18446744073709551616"; "multiple_of_bigfrac_vs_bigint")]
#[test_case(r#"{"type":"number","multipleOf":0.1}"#, "5"; "multiple_of_bigfrac_vs_small_u64")]
#[test_case(r#"{"type":"number","multipleOf":0.1}"#, "-5"; "multiple_of_bigfrac_vs_negative_i64")]
// Strict comparisons against a bound larger than u64, across instance shapes.
#[test_case(r#"{"type":"number","exclusiveMinimum":18446744073709551616}"#, "36893488147419103232"; "exclusive_minimum_bigint_vs_bigint")]
#[test_case(r#"{"type":"number","exclusiveMinimum":18446744073709551616}"#, "18446744073709551616"; "exclusive_minimum_bigint_boundary")]
#[test_case(r#"{"type":"number","exclusiveMinimum":18446744073709551616}"#, "5"; "exclusive_minimum_bigint_vs_small_u64")]
#[test_case(r#"{"type":"number","exclusiveMinimum":18446744073709551616}"#, "-5"; "exclusive_minimum_bigint_vs_negative_i64")]
#[test_case(r#"{"type":"number","exclusiveMinimum":18446744073709551616}"#, "0.5"; "exclusive_minimum_bigint_vs_fractional")]
#[test_case(r#"{"type":"number","exclusiveMaximum":18446744073709551616}"#, "5"; "exclusive_maximum_bigint_vs_small_u64")]
#[test_case(r#"{"type":"number","exclusiveMaximum":18446744073709551616}"#, "1e20"; "exclusive_maximum_bigint_vs_bigint")]
#[test_case(r#"{"type":"number","exclusiveMaximum":18446744073709551616}"#, "-5"; "exclusive_maximum_bigint_vs_negative_i64")]
#[test_case(r#"{"type":"number","exclusiveMaximum":18446744073709551616}"#, "0.5"; "exclusive_maximum_bigint_vs_fractional")]
// Non-strict comparisons against a fractional bound, across instance shapes.
#[test_case(r#"{"type":"number","minimum":0.1}"#, "0.05"; "minimum_bigfrac_vs_fraction_below")]
#[test_case(r#"{"type":"number","minimum":0.1}"#, "5"; "minimum_bigfrac_vs_small_u64")]
#[test_case(r#"{"type":"number","minimum":0.1}"#, "-5"; "minimum_bigfrac_vs_negative_i64")]
#[test_case(r#"{"type":"number","minimum":0.1}"#, "1e20"; "minimum_bigfrac_vs_big_integer")]
#[test_case(r#"{"type":"number","maximum":0.1}"#, "0.05"; "maximum_bigfrac_vs_fraction_below")]
#[test_case(r#"{"type":"number","maximum":0.1}"#, "5"; "maximum_bigfrac_vs_small_u64")]
#[test_case(r#"{"type":"number","maximum":0.1}"#, "-5"; "maximum_bigfrac_vs_negative_i64")]
#[test_case(r#"{"type":"number","maximum":0.1}"#, "1e20"; "maximum_bigfrac_vs_big_integer")]
// Bounds and a divisor whose exponents overflow or underflow f64.
#[test_case(r#"{"type":"number","exclusiveMinimum":1e-2000000}"#, "1"; "tiny_exclusive_minimum_vs_one")]
#[test_case(r#"{"type":"number","exclusiveMinimum":1e-2000000}"#, "0"; "tiny_exclusive_minimum_vs_zero")]
#[test_case(r#"{"type":"number","exclusiveMinimum":1e-2000000}"#, "-1"; "tiny_exclusive_minimum_vs_negative")]
#[test_case(r#"{"type":"number","exclusiveMaximum":1e2000000}"#, "1"; "huge_exclusive_maximum_vs_one")]
#[test_case(r#"{"type":"number","exclusiveMaximum":1e2000000}"#, "1e400"; "huge_exclusive_maximum_vs_big")]
#[test_case(r#"{"type":"number","exclusiveMaximum":1e2000000}"#, "-1"; "huge_exclusive_maximum_vs_negative")]
#[test_case(r#"{"type":"number","multipleOf":1e2000000}"#, "1"; "huge_multiple_of_vs_one")]
#[test_case(r#"{"type":"number","multipleOf":1e2000000}"#, "0"; "huge_multiple_of_vs_zero")]
// Inclusive bounds whose exponent overflows f64, across instance shapes.
#[test_case(r#"{"type":"number","minimum":1e2000000}"#, "1"; "huge_minimum_vs_one")]
#[test_case(r#"{"type":"number","minimum":1e2000000}"#, "1e400"; "huge_minimum_vs_big")]
#[test_case(r#"{"type":"number","maximum":1e2000000}"#, "1"; "huge_maximum_vs_one")]
#[test_case(r#"{"type":"number","maximum":1e2000000}"#, "-1"; "huge_maximum_vs_negative")]
// A bound larger than u64 compared against an instance whose exponent overflows f64.
#[test_case(r#"{"type":"number","minimum":18446744073709551616}"#, "1e2000000"; "minimum_bigint_vs_huge_scientific")]
#[test_case(r#"{"type":"number","maximum":18446744073709551616}"#, "-1e2000000"; "maximum_bigint_vs_huge_negative_scientific")]
// A divisor larger than u64 checked against an integer written with a fractional part.
#[test_case(r#"{"type":"number","multipleOf":18446744073709551616}"#, "8.0"; "multiple_of_bigint_vs_integer_valued_float")]
// Constraints checked against an instance whose exponent overflows f64.
#[test_case(r#"{"type":"number","minimum":0.1}"#, "1e2000000"; "minimum_bigfrac_vs_huge_scientific")]
#[test_case(r#"{"type":"number","multipleOf":18446744073709551616}"#, "1e2000000"; "multiple_of_bigint_vs_huge_scientific")]
#[test_case(r#"{"type":"number","multipleOf":0.1}"#, "1e2000000"; "multiple_of_bigfrac_vs_huge_scientific")]
// A lower bound whose negative exponent overflows f64 accepts every instance.
#[test_case(r#"{"type":"number","minimum":-1e2000000}"#, "0"; "huge_negative_minimum_vs_zero")]
fn test_arbitrary_precision_codegen_matches_runtime(schema_json: &str, instance_json: &str) {
    let schema: serde_json::Value = serde_json::from_str(schema_json).expect("valid schema json");
    let instance: serde_json::Value =
        serde_json::from_str(instance_json).expect("valid instance json");
    let runtime = runtime_valid(&schema, &instance);

    let generated = match schema_json {
        r#"{"type":"number","minimum":18446744073709551616}"# => {
            ArbitraryPrecisionMinimumValidator::is_valid(&instance)
        }
        r#"{"type":"number","maximum":18446744073709551616}"# => {
            ArbitraryPrecisionMaximumValidator::is_valid(&instance)
        }
        r#"{"type":"number","exclusiveMinimum":0.1}"# => {
            ArbitraryPrecisionExclusiveMinimumValidator::is_valid(&instance)
        }
        r#"{"type":"number","exclusiveMaximum":0.1}"# => {
            ArbitraryPrecisionExclusiveMaximumValidator::is_valid(&instance)
        }
        r#"{"type":"number","multipleOf":18446744073709551616}"# => {
            ArbitraryPrecisionMultipleOfBigIntValidator::is_valid(&instance)
        }
        r#"{"type":"number","multipleOf":0.1}"# => {
            ArbitraryPrecisionMultipleOfBigFracValidator::is_valid(&instance)
        }
        r#"{"type":"number","multipleOf":1e400}"# => {
            ArbitraryPrecisionMultipleOfExtremeValidator::is_valid(&instance)
        }
        r#"{"type":"number","exclusiveMinimum":18446744073709551616}"# => {
            ArbitraryPrecisionExclusiveMinimumBigIntValidator::is_valid(&instance)
        }
        r#"{"type":"number","exclusiveMaximum":18446744073709551616}"# => {
            ArbitraryPrecisionExclusiveMaximumBigIntValidator::is_valid(&instance)
        }
        r#"{"type":"number","minimum":0.1}"# => {
            ArbitraryPrecisionMinimumBigFracValidator::is_valid(&instance)
        }
        r#"{"type":"number","maximum":0.1}"# => {
            ArbitraryPrecisionMaximumBigFracValidator::is_valid(&instance)
        }
        r#"{"type":"number","exclusiveMinimum":1e-2000000}"# => {
            ArbitraryPrecisionTinyExclusiveMinimumValidator::is_valid(&instance)
        }
        r#"{"type":"number","exclusiveMaximum":1e2000000}"# => {
            ArbitraryPrecisionHugeExclusiveMaximumValidator::is_valid(&instance)
        }
        r#"{"type":"number","multipleOf":1e2000000}"# => {
            ArbitraryPrecisionHugeMultipleOfValidator::is_valid(&instance)
        }
        r#"{"type":"number","minimum":1e2000000}"# => {
            ArbitraryPrecisionHugeMinimumValidator::is_valid(&instance)
        }
        r#"{"type":"number","maximum":1e2000000}"# => {
            ArbitraryPrecisionHugeMaximumValidator::is_valid(&instance)
        }
        r#"{"type":"number","minimum":-1e2000000}"# => {
            ArbitraryPrecisionHugeNegativeMinimumValidator::is_valid(&instance)
        }
        _ => unreachable!("unknown schema in test matrix"),
    };

    assert_eq!(
        generated, runtime,
        "codegen/runtime mismatch for schema={schema_json}, instance={instance_json}"
    );
}

#[cfg(feature = "arbitrary-precision")]
#[test_case(
    r#"{"type":"number","minimum":18446744073709551616}"#,
    ArbitraryPrecisionMinimumValidator::is_valid;
    "minimum_bigint_vs_huge_non_integer_decimal"
)]
#[test_case(
    r#"{"type":"number","maximum":18446744073709551616}"#,
    ArbitraryPrecisionMaximumValidator::is_valid;
    "maximum_bigint_vs_huge_non_integer_decimal"
)]
#[test_case(
    r#"{"type":"number","exclusiveMinimum":18446744073709551616}"#,
    ArbitraryPrecisionExclusiveMinimumBigIntValidator::is_valid;
    "exclusive_minimum_bigint_vs_huge_non_integer_decimal"
)]
#[test_case(
    r#"{"type":"number","exclusiveMaximum":18446744073709551616}"#,
    ArbitraryPrecisionExclusiveMaximumBigIntValidator::is_valid;
    "exclusive_maximum_bigint_vs_huge_non_integer_decimal"
)]
fn test_bigint_bound_vs_huge_non_integer_decimal(
    schema_json: &str,
    generated: fn(&serde_json::Value) -> bool,
) {
    let instance_json = format!("{}.5", "9".repeat(320));
    let instance: serde_json::Value =
        serde_json::from_str(&instance_json).expect("valid instance json");
    let schema: serde_json::Value = serde_json::from_str(schema_json).expect("valid schema json");
    assert_eq!(generated(&instance), runtime_valid(&schema, &instance));
}

#[test]
fn test_self_ref_schema_does_not_emit_runtime_self_calls() {
    assert!(SelfRefValidator::is_valid(&serde_json::json!({"a": 1})));
    assert!(SelfRefValidator::is_valid(&serde_json::json!(null)));
}

#[test_case(serde_json::json!("x"); "string_instance")]
#[test_case(serde_json::json!({"kind":"a"}); "object_discriminator_a")]
#[test_case(serde_json::json!({"kind":"b"}); "object_discriminator_b")]
#[test_case(serde_json::json!({"kind":"c"}); "object_discriminator_unknown")]
fn test_one_of_discriminator_optimization_keeps_non_object_semantics(instance: serde_json::Value) {
    let schema = serde_json::json!({
        "oneOf": [
            {"required":["kind"],"properties":{"kind":{"const":"a"}}},
            {"required":["kind"],"properties":{"kind":{"const":"b"}}},
            {"type":"string"}
        ]
    });
    assert_is_valid_parity(
        &schema,
        OneOfDiscriminatorWithScalarBranchValidator::is_valid(&instance),
        &instance,
    );
}

// JSON Schema `const: 1` matches `1.0`, so a float-valued integer tag must select
// the same `oneOf` branch as its integer form.
#[test_case(serde_json::json!({"tag": 1, "a": "x"}) ; "integer_form")]
#[test_case(serde_json::json!({"tag": 1.0, "a": "x"}) ; "float_integer_form")]
#[test_case(serde_json::json!({"tag": 2.0, "a": 3}) ; "float_integer_second_branch")]
#[test_case(serde_json::json!({"tag": 1.5}) ; "non_integer_tag")]
#[test_case(serde_json::json!({"tag": 3}) ; "unmatched_tag")]
#[test_case(serde_json::json!({"tag": 1.0, "a": 3}) ; "float_tag_body_mismatch")]
fn test_one_of_int_discriminator_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({
        "oneOf": [
            {"type":"object","required":["tag"],"properties":{"tag":{"const":1},"a":{"type":"string"}}},
            {"type":"object","required":["tag"],"properties":{"tag":{"const":2},"a":{"type":"integer"}}}
        ]
    });
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    let expected = runtime.is_valid(&instance);
    assert_eq!(
        OneOfIntDiscriminatorValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        OneOfIntDiscriminatorValidator::validate(&instance).is_ok(),
        expected
    );
}

// Draft 4 defines only `id`; a modern `$id` key must not shift the base URI.
#[test_case(serde_json::json!({"x": {"in": "text"}}) ; "string_per_root_base")]
#[test_case(serde_json::json!({"x": {"in": 5}}) ; "integer_rejected_per_root_base")]
fn test_draft4_ignores_modern_dollar_id(instance: serde_json::Value) {
    let schema = serde_json::json!(
        {"properties":{"x":{"$id":"http://example.com/nested/anchor.json","properties":{"in":{"$ref":"item.json"}}}}}
    );
    let anchor = serde_json::json!({"type":"object"});
    let root_item = serde_json::json!({"type":"string"});
    let nested_item = serde_json::json!({"type":"integer"});
    let registry = jsonschema::Registry::new()
        .add("http://example.com/nested/anchor.json", &anchor)
        .expect("resource accepted")
        .add("json-schema:///root/item.json", &root_item)
        .expect("resource accepted")
        .add("http://example.com/nested/item.json", &nested_item)
        .expect("resource accepted")
        .prepare()
        .expect("registry build failed");
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft4)
        .with_base_uri("json-schema:///root/main.json")
        .with_registry(&registry)
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        Draft4ModernIdIgnoredValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(serde_json::json!("abc"), true ; "string_branch")]
#[test_case(serde_json::json!(42), true ; "integer_branch")]
#[test_case(serde_json::json!(true), false ; "bool_rejected")]
fn test_external_ref_fragments_use_distinct_helpers(instance: serde_json::Value, expected: bool) {
    assert_eq!(ExternalRefFragmentsValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!(1), true ; "const_value")]
#[test_case(serde_json::json!("anything"), true ; "other_value")]
fn test_draft4_ignores_const_keyword(instance: serde_json::Value, expected: bool) {
    assert_eq!(Draft4ConstIgnoredValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!({"a": 1}), true ; "required present")]
#[test_case(serde_json::json!({}), false ; "required missing")]
fn test_required_tracking_when_properties_are_trivial(instance: serde_json::Value, expected: bool) {
    assert_eq!(RequiredInPropertiesValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!(0.3), true ; "0_3_is_multiple_of_0_1")]
#[test_case(serde_json::json!(0.35), false ; "0_35_not_multiple_of_0_1")]
fn test_multiple_of_decimal_matches_runtime_semantics(instance: serde_json::Value, expected: bool) {
    assert_eq!(DecimalMultipleOfValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!(4), true ; "above_bound_accepted")]
#[test_case(serde_json::json!(3), false ; "equal_to_bound_rejected")]
#[test_case(serde_json::json!(2), false ; "below_bound_rejected")]
fn test_integer_exclusive_minimum_matches_runtime(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({"type":"number","exclusiveMinimum":3});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        IntegerExclusiveMinimumValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!([]), true ; "empty_array")]
#[test_case(serde_json::json!([[]]), true ; "nested_empty_array")]
#[test_case(serde_json::json!([1]), false ; "array_with_scalar_fails")]
fn test_recursive_ref_constraints_are_not_compiled_away(
    instance: serde_json::Value,
    expected: bool,
) {
    assert_eq!(RecursiveNodeValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!(1), false ; "number_rejected")]
#[test_case(serde_json::json!("x"), false ; "string_rejected")]
fn test_ref_siblings_are_enforced_in_draft_201909(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        RefSiblingDraft201909Validator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!(1), false ; "number_rejected")]
#[test_case(serde_json::json!("x"), false ; "string_rejected")]
fn test_ref_siblings_are_enforced_in_draft_202012(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        RefSiblingDraft202012Validator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!({"foo": 1, "bar": 2}), true ; "dependent_schema_satisfied")]
#[test_case(serde_json::json!({"foo": 1}), false ; "dependent_schema_missing_bar")]
#[test_case(serde_json::json!({"bar": 2}), true ; "dependent_schema_not_triggered")]
fn test_dependent_schemas_draft_201909(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        DependentSchemasDraft201909Validator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!([1, 2]), true ; "within_min_max")]
#[test_case(serde_json::json!([1]), false ; "below_min")]
#[test_case(serde_json::json!([1, 2, 3, 4]), false ; "above_max")]
#[test_case(serde_json::json!(["x", 1, 2]), true ; "counts_only_matching_items")]
fn test_contains_min_max_bounds_draft_201909(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        ContainsBoundsDraft201909Validator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!("not an ip"), true ; "default_off_ignores_format")]
#[test_case(serde_json::json!("127.0.0.1"), true ; "default_off_still_accepts_valid")]
fn test_draft_201909_format_validation_default_off(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        Draft201909FormatDefaultOffValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!("not an ip"), false ; "enabled_rejects_invalid_format")]
#[test_case(serde_json::json!("127.0.0.1"), true ; "enabled_accepts_valid_format")]
fn test_draft_201909_format_validation_override(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        Draft201909FormatEnabledValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!("user@example.com"); "valid_email")]
#[test_case(serde_json::json!("user@localhost"); "missing_tld")]
#[test_case(serde_json::json!("Name <user@example.com>"); "display_text_disallowed")]
#[test_case(serde_json::json!("user@[127.0.0.1]"); "domain_literal_disallowed")]
fn test_email_options_codegen_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"string","format":"email"});
    let runtime = jsonschema::options()
        .with_draft(referencing::Draft::Draft7)
        .should_validate_formats(true)
        .with_email_options(
            jsonschema::EmailOptions::default()
                .with_required_tld()
                .without_domain_literal()
                .without_display_text(),
        )
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        EmailOptionsConfiguredValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(serde_json::json!("user@example.com"); "two_segment_domain")]
#[test_case(serde_json::json!("user@localhost"); "single_segment_domain")]
#[test_case(serde_json::json!("user@sub.example.com"); "three_segment_domain")]
fn test_email_options_sub_domain_variants_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"string","format":"email"});

    let runtime = jsonschema::options()
        .with_draft(referencing::Draft::Draft7)
        .should_validate_formats(true)
        .with_email_options(jsonschema::EmailOptions::default().with_minimum_sub_domains(2))
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        EmailOptionsMinimumSubDomainsValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );

    let runtime = jsonschema::options()
        .with_draft(referencing::Draft::Draft7)
        .should_validate_formats(true)
        .with_email_options(
            jsonschema::EmailOptions::default()
                .with_no_minimum_sub_domains()
                .without_domain_literal()
                .without_display_text(),
        )
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        EmailOptionsNoMinimumSubDomainsValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(serde_json::json!(1); "valid_integer")]
#[test_case(serde_json::json!(0); "minimum_violation")]
#[test_case(serde_json::json!("x"); "wrong_type")]
fn test_base_uri_codegen_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$ref":"defs.json#/$defs/item"});
    let defs_schema = serde_json::json!({"$defs":{"item":{"type":"integer","minimum":1}}});
    let defs_registry = jsonschema::Registry::new()
        .add("json-schema:///root/defs.json", &defs_schema)
        .expect("resource accepted")
        .prepare()
        .expect("registry build failed");
    let runtime = jsonschema::options()
        .with_base_uri("json-schema:///root/main.json")
        .with_registry(&defs_registry)
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        BaseUriRelativeRefValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(serde_json::json!("$12"), true ; "custom_format_accepts_matching")]
#[test_case(serde_json::json!("12"), false ; "custom_format_rejects_non_matching")]
#[test_case(serde_json::json!(12), false ; "type_still_applies")]
fn test_custom_format_with_draft7_default_validation(instance: serde_json::Value, expected: bool) {
    assert_eq!(CustomFormatDraft7Validator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!("12"), true ; "draft2019_default_off_ignores_custom")]
#[test_case(serde_json::json!("$12"), true ; "draft2019_default_off_accepts_any_string")]
fn test_custom_format_is_ignored_by_default_in_draft201909(
    instance: serde_json::Value,
    expected: bool,
) {
    assert_eq!(
        CustomFormatDraft201909DefaultOffValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!("$12"), true ; "draft2019_enabled_accepts_matching")]
#[test_case(serde_json::json!("12"), false ; "draft2019_enabled_rejects_non_matching")]
fn test_custom_format_validation_override_in_draft201909(
    instance: serde_json::Value,
    expected: bool,
) {
    assert_eq!(
        CustomFormatDraft201909EnabledValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!("x"), true ; "custom_overrides_builtin_accepts_x")]
#[test_case(serde_json::json!("foo@example.com"), false ; "custom_overrides_builtin_rejects_email")]
fn test_custom_format_overrides_builtin(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        CustomFormatOverrideBuiltInValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!("anything"), true ; "unknown_format_ignored")]
#[test_case(serde_json::json!(1), false ; "type_still_enforced_for_unknown_format")]
fn test_unknown_format_ignored_when_configured(instance: serde_json::Value, expected: bool) {
    assert_eq!(UnknownFormatIgnoredValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!("x"), true ; "sibling_min_length_satisfied_format_ignored")]
#[test_case(serde_json::json!(""), false ; "sibling_min_length_enforced")]
fn test_unknown_format_ignored_with_string_sibling(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        UnknownFormatIgnoredWithSiblingValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!(1), true ; "number_valid_when_format_ignored")]
#[test_case(serde_json::json!("x"), false ; "string_rejected_by_type_when_format_ignored")]
fn test_unknown_format_does_not_widen_type_in_draft201909(
    instance: serde_json::Value,
    expected: bool,
) {
    assert_eq!(
        NumberTypeUnknownFormatDraft201909Validator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!(1), true ; "number_valid_when_unknown_format_ignored")]
#[test_case(serde_json::json!("x"), false ; "string_rejected_by_type_with_unknown_format_ignored")]
fn test_unknown_format_does_not_widen_type_in_draft7(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        NumberTypeUnknownFormatDraft7Validator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!({"foo": 1}), true ; "ignored_when_keyword_unsupported")]
#[test_case(serde_json::json!({"foo": 1, "bar": 2}), true ; "also_valid_when_present")]
fn test_dependent_required_is_ignored_in_draft7(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        Draft7DependentRequiredIgnoredValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!(1), true ; "draft4_if_then_ignored")]
#[test_case(serde_json::json!("x"), true ; "draft4_if_then_ignored_other_value")]
fn test_if_then_is_ignored_in_draft4_codegen(instance: serde_json::Value, expected: bool) {
    assert_eq!(Draft4IfThenIgnoredValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!(1), true ; "draft6_if_then_ignored")]
#[test_case(serde_json::json!("x"), true ; "draft6_if_then_ignored_other_value")]
fn test_if_then_is_ignored_in_draft6_codegen(instance: serde_json::Value, expected: bool) {
    assert_eq!(Draft6IfThenIgnoredValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!("ok"), true ; "string_valid")]
#[test_case(serde_json::json!({"foo":1}), false ; "object_rejected_by_type")]
fn test_dependent_schemas_is_ignored_in_draft7_typed_dispatch(
    instance: serde_json::Value,
    expected: bool,
) {
    assert_eq!(
        Draft7DependentSchemasIgnoredWithTypeValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!({"y": 1}), true ; "draft4_property_names_ignored")]
#[test_case(serde_json::json!({"x": 1}), true ; "draft4_property_names_ignored_even_when_matching")]
fn test_property_names_is_ignored_in_draft4_codegen(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        Draft4PropertyNamesIgnoredValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!("not base64"), true ; "draft4_content_encoding_ignored")]
#[test_case(serde_json::json!(1), false ; "type_still_enforced")]
fn test_content_keywords_are_ignored_in_draft4_codegen(
    instance: serde_json::Value,
    expected: bool,
) {
    assert_eq!(
        Draft4ContentEncodingIgnoredValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!([]), true ; "draft4_contains_ignored_empty")]
#[test_case(serde_json::json!(["x"]), true ; "draft4_contains_ignored_non_matching")]
fn test_contains_is_ignored_in_draft4_codegen(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        Draft4ContainsIgnoredValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!([1]), true ; "draft7_min_contains_ignored")]
#[test_case(serde_json::json!(["x"]), false ; "contains_still_applies")]
fn test_min_contains_is_ignored_before_201909_codegen(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        Draft7MinContainsIgnoredValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!(["x", 2]), true ; "first_item_string_is_valid")]
#[test_case(serde_json::json!([1, 2]), false ; "first_item_non_string_is_invalid")]
#[test_case(serde_json::json!("not array"), false ; "adjacent_type_still_applies")]
fn test_cross_draft_ref_uses_referenced_prefix_items_semantics(
    instance: serde_json::Value,
    expected: bool,
) {
    assert_eq!(
        Draft201909RefTo202012PrefixItemsValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!({}), true ; "empty_object")]
#[test_case(serde_json::json!({"child": {}}), true ; "nested_object")]
#[test_case(serde_json::json!({"child": 1}), false ; "non_object_child")]
fn test_recursive_ref_keyword_in_draft_201909(instance: serde_json::Value, expected: bool) {
    assert_eq!(
        RecursiveRefDraft201909Validator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!(0), true ; "zero_is_valid")]
#[test_case(serde_json::json!(1), true ; "positive_integer_is_valid")]
#[test_case(serde_json::json!(-1), false ; "below_minimum_integer_is_invalid")]
#[test_case(serde_json::json!(1.5), false ; "fractional_number_is_invalid")]
#[test_case(serde_json::json!("1"), false ; "wrong_type_is_invalid")]
fn test_numeric_keywords_respect_integer_only_type(instance: serde_json::Value, expected: bool) {
    assert_eq!(IntegerMinimumValidator::is_valid(&instance), expected);
}

#[test_case(serde_json::json!("abc"), true ; "string_type_branch_is_valid")]
#[test_case(serde_json::json!(0), true ; "integer_at_max_is_valid")]
#[test_case(serde_json::json!(-1), true ; "integer_below_max_is_valid")]
#[test_case(serde_json::json!(1), false ; "integer_above_max_is_invalid")]
#[test_case(serde_json::json!(1.5), false ; "fractional_number_is_invalid_even_with_numeric_keyword")]
#[test_case(serde_json::json!(true), false ; "non_declared_type_is_invalid")]
fn test_numeric_keywords_do_not_widen_integer_in_union_types(
    instance: serde_json::Value,
    expected: bool,
) {
    assert_eq!(
        IntegerOrStringMaximumValidator::is_valid(&instance),
        expected
    );
}

#[test_case(serde_json::json!({"foo": 1, "bar": 2}), false ; "dependency_schema_does_not_evaluate_bar")]
#[test_case(serde_json::json!({"foo": 1}), true ; "no_extra_properties")]
#[test_case(serde_json::json!({"foo": 1, "baz": 2}), false ; "baz_is_unevaluated")]
#[test_case(serde_json::json!({"bar": 2}), false ; "bar_unevaluated_when_dependency_not_triggered")]
fn test_dependencies_affect_unevaluated_properties_like_runtime(
    instance: serde_json::Value,
    expected: bool,
) {
    let schema = serde_json::json!({
        "type":"object",
        "properties":{"foo":true},
        "dependencies":{"foo":{"properties":{"bar":true}}},
        "unevaluatedProperties":false
    });
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft201909)
        .build(&schema)
        .expect("schema should build");

    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        DependenciesUnevaluatedPropertiesValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        DependenciesUnevaluatedPropertiesValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(jsonschema::Draft::Draft4, serde_json::json!(1), true ; "draft4_const_value")]
#[test_case(
    jsonschema::Draft::Draft4,
    serde_json::json!("anything"),
    true
; "draft4_const_is_ignored"
)]
fn test_runtime_validator_draft_based_const_behavior(
    draft: jsonschema::Draft,
    instance: serde_json::Value,
    expected: bool,
) {
    let schema = serde_json::json!({"const": 1});
    let validator = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("schema should build");

    assert_eq!(validator.is_valid(&instance), expected);
}

#[test_case(jsonschema::Draft::Draft201909 ; "runtime_2019_09_ref_siblings")]
#[test_case(jsonschema::Draft::Draft202012 ; "runtime_2020_12_ref_siblings")]
fn test_runtime_validator_ref_sibling_behavior(draft: jsonschema::Draft) {
    let schema = serde_json::json!({"$ref":"urn:num","type":"string"});
    let num_schema = serde_json::json!({"type":"number"});
    let num_registry = jsonschema::Registry::new()
        .add("urn:num", &num_schema)
        .expect("resource accepted")
        .prepare()
        .expect("registry build failed");
    let validator = jsonschema::options()
        .with_draft(draft)
        .with_registry(&num_registry)
        .build(&schema)
        .expect("schema should build");

    assert!(!validator.is_valid(&serde_json::json!(1)));
    assert!(!validator.is_valid(&serde_json::json!("x")));
}

#[test_case(
    jsonschema::Draft::Draft4,
    serde_json::json!({"if":{"const":1},"then":false}),
    serde_json::json!(1),
    true
; "runtime_draft4_if_then_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft6,
    serde_json::json!({"if":{"const":1},"then":false}),
    serde_json::json!(1),
    true
; "runtime_draft6_if_then_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft7,
    serde_json::json!({"type":"string","dependentSchemas":{"foo":{"required":["bar"]}}}),
    serde_json::json!({"foo":1}),
    false
; "runtime_draft7_dependent_schemas_ignored_type_applies"
)]
#[test_case(
    jsonschema::Draft::Draft4,
    serde_json::json!({"propertyNames":{"pattern":"^x"}}),
    serde_json::json!({"y":1}),
    true
; "runtime_draft4_property_names_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft4,
    serde_json::json!({"type":"string","contentEncoding":"base64"}),
    serde_json::json!("not base64"),
    true
; "runtime_draft4_content_encoding_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft7,
    serde_json::json!({"type":"array","contains":{"type":"integer"},"minContains":2}),
    serde_json::json!([1]),
    true
; "runtime_draft7_min_contains_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft7,
    serde_json::json!({"type":"integer","minimum":0}),
    serde_json::json!(1.5),
    false
; "runtime_integer_type_rejects_fractional_numbers_with_numeric_keywords"
)]
#[test_case(
    jsonschema::Draft::Draft7,
    serde_json::json!({"type":["integer","string"],"maximum":0}),
    serde_json::json!(1.5),
    false
; "runtime_union_with_integer_and_numeric_keywords_rejects_fractional_numbers"
)]
fn test_runtime_validator_draft_keyword_gating(
    draft: jsonschema::Draft,
    schema: serde_json::Value,
    instance: serde_json::Value,
    expected: bool,
) {
    let validator = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("schema should build");
    assert_eq!(validator.is_valid(&instance), expected);
}

#[test_case(
    serde_json::json!("a"),
    true
; "string_short_still_valid_when_validation_vocab_disabled"
)]
#[test_case(
    serde_json::json!(1),
    true
; "non_string_still_valid_when_validation_vocab_disabled"
)]
fn test_validation_vocabulary_gating_parity(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({
        "$schema": "json-schema:///meta/no-validation",
        "type": "string",
        "minLength": 2,
        "enum": ["ab"]
    });
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-validation",
            serde_json::json!({
                "$id": "json-schema:///meta/no-validation",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$vocabulary": {
                    "https://json-schema.org/draft/2020-12/vocab/core": true,
                    "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                    "https://json-schema.org/draft/2020-12/vocab/validation": false,
                    "https://json-schema.org/draft/2020-12/vocab/unevaluated": true,
                    "https://json-schema.org/draft/2020-12/vocab/format-annotation": true
                }
            }),
        )],
    );

    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        NoValidationVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoValidationVocabularyValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(
    serde_json::json!({"a": 1, "b": 2}),
    true
; "properties_additional_allof_are_ignored_when_applicator_vocab_disabled"
)]
#[test_case(
    serde_json::json!({}),
    true
; "empty_object_still_valid_when_only_type_applies"
)]
#[test_case(
    serde_json::json!(1),
    false
; "type_keyword_from_validation_vocab_still_applies"
)]
fn test_applicator_vocabulary_gating_parity(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({
        "$schema": "json-schema:///meta/no-applicator",
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "additionalProperties": false,
        "allOf": [{"required": ["a"]}]
    });
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-applicator",
            serde_json::json!({
                "$id": "json-schema:///meta/no-applicator",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$vocabulary": {
                    "https://json-schema.org/draft/2020-12/vocab/core": true,
                    "https://json-schema.org/draft/2020-12/vocab/applicator": false,
                    "https://json-schema.org/draft/2020-12/vocab/validation": true,
                    "https://json-schema.org/draft/2020-12/vocab/unevaluated": true,
                    "https://json-schema.org/draft/2020-12/vocab/format-annotation": true
                }
            }),
        )],
    );

    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        NoApplicatorVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoApplicatorVocabularyValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(
    serde_json::json!({"a":"ok","extra":1}),
    true
; "unevaluated_properties_ignored_when_unevaluated_vocab_disabled"
)]
#[test_case(
    serde_json::json!({"a": 1}),
    false
; "properties_still_apply_with_applicator_vocab_enabled"
)]
fn test_unevaluated_vocabulary_gating_parity(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({
        "$schema": "json-schema:///meta/no-unevaluated",
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "unevaluatedProperties": false
    });
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-unevaluated",
            serde_json::json!({
                "$id": "json-schema:///meta/no-unevaluated",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$vocabulary": {
                    "https://json-schema.org/draft/2020-12/vocab/core": true,
                    "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                    "https://json-schema.org/draft/2020-12/vocab/validation": true,
                    "https://json-schema.org/draft/2020-12/vocab/unevaluated": false,
                    "https://json-schema.org/draft/2020-12/vocab/format-annotation": true
                }
            }),
        )],
    );

    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        NoUnevaluatedVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoUnevaluatedVocabularyValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(
    serde_json::json!(["x"]),
    true
; "array_items_and_contains_ignored_when_applicator_vocab_disabled"
)]
#[test_case(
    serde_json::json!([1]),
    true
; "non_string_items_allowed_when_applicator_vocab_disabled"
)]
#[test_case(
    serde_json::json!("not array"),
    false
; "type_still_enforced_with_validation_vocab_enabled"
)]
fn test_array_applicator_vocabulary_gating_parity(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({
        "$schema": "json-schema:///meta/no-applicator",
        "type": "array",
        "items": {"type": "string"},
        "contains": {"type": "integer"}
    });
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-applicator",
            serde_json::json!({
                "$id": "json-schema:///meta/no-applicator",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$vocabulary": {
                    "https://json-schema.org/draft/2020-12/vocab/core": true,
                    "https://json-schema.org/draft/2020-12/vocab/applicator": false,
                    "https://json-schema.org/draft/2020-12/vocab/validation": true,
                    "https://json-schema.org/draft/2020-12/vocab/unevaluated": true,
                    "https://json-schema.org/draft/2020-12/vocab/format-annotation": true
                }
            }),
        )],
    );

    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        NoApplicatorArrayVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoApplicatorArrayVocabularyValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[test_case(
    serde_json::json!(["ok", 1]),
    true
; "unevaluated_items_ignored_when_unevaluated_vocab_disabled"
)]
#[test_case(
    serde_json::json!([1]),
    false
; "prefix_items_still_applies_via_applicator_vocab"
)]
fn test_array_unevaluated_vocabulary_gating_parity(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({
        "$schema": "json-schema:///meta/no-unevaluated",
        "type": "array",
        "prefixItems": [{"type": "string"}],
        "unevaluatedItems": false
    });
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-unevaluated",
            serde_json::json!({
                "$id": "json-schema:///meta/no-unevaluated",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$vocabulary": {
                    "https://json-schema.org/draft/2020-12/vocab/core": true,
                    "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                    "https://json-schema.org/draft/2020-12/vocab/validation": true,
                    "https://json-schema.org/draft/2020-12/vocab/unevaluated": false,
                    "https://json-schema.org/draft/2020-12/vocab/format-annotation": true
                }
            }),
        )],
    );

    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        NoUnevaluatedArrayVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoUnevaluatedArrayVocabularyValidator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(
    schema = r##"{
        "$id": "json-schema:///root",
        "type": "object",
        "properties": {
            "child": {"$recursiveRef": "#"}
        }
    }"##,
    draft = referencing::Draft::Draft201909
)]
struct RecursiveRefPathValidator;

#[test]
fn test_recursive_ref_validate_emits_proper_schema_path() {
    // The innermost violation is at `child.child = 42`, which fails the top-level
    // `type: object` constraint at schema path `/type`.
    let invalid = serde_json::json!({"child": {"child": 42}});
    let err = RecursiveRefPathValidator::validate(&invalid).expect_err("should fail");
    let codegen_sp = err.schema_path().to_string();

    // Compare against runtime engine for the same schema.
    let schema: serde_json::Value = serde_json::from_str(
        r##"{
            "$id": "json-schema:///root",
            "type": "object",
            "properties": {
                "child": {"$recursiveRef": "#"}
            }
        }"##,
    )
    .unwrap();
    let runtime_v = jsonschema::options()
        .with_draft(referencing::Draft::Draft201909)
        .build(&schema)
        .expect("runtime build ok");
    let runtime_err = runtime_v
        .validate(&invalid)
        .expect_err("runtime should fail");
    let runtime_sp = runtime_err.schema_path().to_string();

    assert_eq!(
        codegen_sp, runtime_sp,
        "codegen schema_path ({codegen_sp:?}) must match runtime ({runtime_sp:?})"
    );
}

#[jsonschema::validator(
    schema = r##"{
        "$id": "json-schema:///dynroot",
        "$defs": {
            "tree": {
                "$dynamicAnchor": "node",
                "type": "object",
                "properties": {
                    "value": {"type": "integer"},
                    "child": {"$dynamicRef": "#node"}
                }
            }
        },
        "$ref": "#/$defs/tree"
    }"##,
    draft = referencing::Draft::Draft202012
)]
struct DynamicRefPathValidator;

// Mutual recursion via two $defs that reference each other.
#[jsonschema::validator(
    schema = r##"{
        "$defs": {
            "a": {
                "type": "object",
                "required": ["next"],
                "properties": {"next": {"$ref": "#/$defs/b"}}
            },
            "b": {
                "type": "object",
                "required": ["next"],
                "properties": {"next": {"$ref": "#/$defs/a"}}
            }
        },
        "$ref": "#/$defs/a"
    }"##,
    draft = referencing::Draft::Draft7
)]
struct MutualRefPathValidator;

#[test]
fn test_mutual_ref_validate_schema_path_matches_runtime() {
    let invalid = serde_json::json!({"next": {"next": 42}});
    let codegen_err = MutualRefPathValidator::validate(&invalid).expect_err("should fail");
    let codegen_sp = codegen_err.schema_path().to_string();

    let schema: serde_json::Value = serde_json::from_str(
        r##"{
            "$defs": {
                "a": {
                    "type": "object",
                    "required": ["next"],
                    "properties": {"next": {"$ref": "#/$defs/b"}}
                },
                "b": {
                    "type": "object",
                    "required": ["next"],
                    "properties": {"next": {"$ref": "#/$defs/a"}}
                }
            },
            "$ref": "#/$defs/a"
        }"##,
    )
    .unwrap();
    let runtime_v = jsonschema::options()
        .with_draft(referencing::Draft::Draft7)
        .build(&schema)
        .expect("runtime build ok");
    let runtime_err = runtime_v
        .validate(&invalid)
        .expect_err("runtime should fail");
    let runtime_sp = runtime_err.schema_path().to_string();

    assert_eq!(codegen_sp, runtime_sp, "codegen vs runtime schema_path");
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"a":{"$ref":"#/$defs/b"},"b":{"$ref":"#/$defs/a"}},"$ref":"#/$defs/a"}"##,
    draft = Draft202012
)]
struct PureMutualRefCycleValidator;

#[jsonschema::validator(
    schema = r##"{"$defs":{"x":{"allOf":[{"$ref":"#/$defs/x"}]}},"$ref":"#/$defs/x"}"##,
    draft = Draft202012
)]
struct AllOfSelfRefCycleValidator;

#[jsonschema::validator(
    schema = r##"{"$defs":{"x":{"anyOf":[{"$ref":"#/$defs/x"}]}},"$ref":"#/$defs/x"}"##,
    draft = Draft202012
)]
struct AnyOfSelfRefCycleValidator;

#[jsonschema::validator(
    schema = r##"{"$defs":{"a":{"$ref":"#/$defs/b"},"b":{"$ref":"#/$defs/c"},"c":{"$ref":"#/$defs/a"}},"$ref":"#/$defs/a"}"##,
    draft = Draft202012
)]
struct TriangleRefCycleValidator;

#[test_case(serde_json::json!(42) ; "integer")]
#[test_case(serde_json::json!("string") ; "string")]
#[test_case(serde_json::json!(null) ; "null")]
#[test_case(serde_json::json!({"nested": [1, 2, 3]}) ; "object")]
fn test_pure_ref_cycles_match_runtime(instance: serde_json::Value) {
    assert_validate_parity_for(
        &serde_json::json!({"$defs":{"a":{"$ref":"#/$defs/b"},"b":{"$ref":"#/$defs/a"}},"$ref":"#/$defs/a"}),
        PureMutualRefCycleValidator::is_valid(&instance),
        PureMutualRefCycleValidator::validate(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &serde_json::json!({"$defs":{"x":{"allOf":[{"$ref":"#/$defs/x"}]}},"$ref":"#/$defs/x"}),
        AllOfSelfRefCycleValidator::is_valid(&instance),
        AllOfSelfRefCycleValidator::validate(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &serde_json::json!({"$defs":{"x":{"anyOf":[{"$ref":"#/$defs/x"}]}},"$ref":"#/$defs/x"}),
        AnyOfSelfRefCycleValidator::is_valid(&instance),
        AnyOfSelfRefCycleValidator::validate(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &serde_json::json!({"$defs":{"a":{"$ref":"#/$defs/b"},"b":{"$ref":"#/$defs/c"},"c":{"$ref":"#/$defs/a"}},"$ref":"#/$defs/a"}),
        TriangleRefCycleValidator::is_valid(&instance),
        TriangleRefCycleValidator::validate(&instance),
        &instance,
    );
}

#[test]
fn test_dynamic_ref_validate_emits_proper_schema_path() {
    let invalid = serde_json::json!({"value": 1, "child": {"value": "not-int"}});
    let err = DynamicRefPathValidator::validate(&invalid).expect_err("should fail");
    let codegen_sp = err.schema_path().to_string();

    let schema: serde_json::Value = serde_json::from_str(
        r##"{
            "$id": "json-schema:///dynroot",
            "$defs": {
                "tree": {
                    "$dynamicAnchor": "node",
                    "type": "object",
                    "properties": {
                        "value": {"type": "integer"},
                        "child": {"$dynamicRef": "#node"}
                    }
                }
            },
            "$ref": "#/$defs/tree"
        }"##,
    )
    .unwrap();
    let runtime_v = jsonschema::options()
        .with_draft(referencing::Draft::Draft202012)
        .build(&schema)
        .expect("runtime build ok");
    let runtime_err = runtime_v
        .validate(&invalid)
        .expect_err("runtime should fail");
    let runtime_sp = runtime_err.schema_path().to_string();

    assert_eq!(
        codegen_sp, runtime_sp,
        "codegen schema_path ({codegen_sp:?}) must match runtime ({runtime_sp:?})"
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"array","prefixItems":[{"type":"integer"},{"type":"string"}],"items":false}"#,
    draft = Draft202012
)]
struct PrefixItemsOrderValidator;

#[test_case(serde_json::json!([1, 9, 0.0]) ; "prefix_and_items_both_fail")]
#[test_case(serde_json::json!([1, "x", true]) ; "items_fails_after_valid_prefix")]
#[test_case(serde_json::json!([true]) ; "prefix_fails_alone")]
fn test_prefix_items_error_order_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&serde_json::json!({
            "type": "array",
            "prefixItems": [{"type": "integer"}, {"type": "string"}],
            "items": false
        }))
        .expect("valid schema");
    assert_validate_parity(
        PrefixItemsOrderValidator::is_valid(&instance),
        PrefixItemsOrderValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"array","items":{"type":"integer"},"minItems":1,"maxItems":5,"uniqueItems":true}"#,
    draft = Draft7
)]
struct ArrayKeywordOrderValidator;

#[test_case(serde_json::json!(["a", "a"]) ; "unique_and_items_both_fail")]
#[test_case(serde_json::json!([0, 1, 2, 3, 4, "a"]) ; "max_and_items_both_fail")]
#[test_case(serde_json::json!(["a"]) ; "items_fails_alone")]
fn test_array_keyword_error_order_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&serde_json::json!({
            "type": "array",
            "items": {"type": "integer"},
            "minItems": 1,
            "maxItems": 5,
            "uniqueItems": true
        }))
        .expect("valid schema");
    assert_validate_parity(
        ArrayKeywordOrderValidator::is_valid(&instance),
        ArrayKeywordOrderValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","minProperties":1,"maxProperties":2}"#,
    draft = Draft7
)]
struct ObjectCountBoundsValidator;

#[test_case(serde_json::json!({}) ; "below_minimum")]
#[test_case(serde_json::json!({"a":1,"b":2,"c":3}) ; "above_maximum")]
fn test_object_count_bounds_error_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&serde_json::json!({"type": "object", "minProperties": 1, "maxProperties": 2}))
        .expect("valid schema");
    assert_validate_parity(
        ObjectCountBoundsValidator::is_valid(&instance),
        ObjectCountBoundsValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

// Overlapping properties and patternProperties on the same object.

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"xkey":{"type":"string"}},"patternProperties":{"^x":{"minLength":3}}}"#
)]
struct PropAndPattern;

#[test_case(serde_json::json!({"xkey":"abcd"}) ; "prop_pattern_both_ok")]
#[test_case(serde_json::json!({"xkey":"ab"}) ; "prop_ok_pattern_fails")]
#[test_case(serde_json::json!({"xkey":5}) ; "prop_type_fails")]
#[test_case(serde_json::json!({"x1":"abc"}) ; "pattern_only_covered")]
#[test_case(serde_json::json!({"other":1}) ; "uncovered_no_ap")]
fn single_pass_prop_and_pattern(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"xkey":{"type":"string"}},"patternProperties":{"^x":{"minLength":3}}});
    assert_is_valid_parity(&schema, PropAndPattern::is_valid(&instance), &instance);
}

#[jsonschema::validator(
    schema = r#"{"type":"object","patternProperties":{"^ab":{"minLength":4},"b$":{"maxLength":6}}}"#
)]
struct MultiPattern;

#[test_case(serde_json::json!({"ab":"abcdef"}) ; "two_patterns_both_ok")]
#[test_case(serde_json::json!({"ab":"abc"}) ; "two_patterns_min_fails")]
#[test_case(serde_json::json!({"ab":"abcdefg"}) ; "two_patterns_max_fails")]
#[test_case(serde_json::json!({"xy":"z"}) ; "no_pattern_match")]
fn single_pass_multi_pattern(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","patternProperties":{"^ab":{"minLength":4},"b$":{"maxLength":6}}});
    assert_is_valid_parity(&schema, MultiPattern::is_valid(&instance), &instance);
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"string"}},"patternProperties":{"^x-":{"type":"string"}},"additionalProperties":{"type":"integer"}}"#
)]
struct UncoveredToApSchema;

#[test_case(serde_json::json!({"a":"s","x-1":"t","other":5}) ; "additional_is_integer")]
#[test_case(serde_json::json!({"a":"s","x-1":"t","other":"no"}) ; "additional_not_integer")]
#[test_case(serde_json::json!({"a":"s","x-1":"t"}) ; "all_covered")]
fn single_pass_uncovered_ap_schema(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"string"}},"patternProperties":{"^x-":{"type":"string"}},"additionalProperties":{"type":"integer"}});
    assert_is_valid_parity(&schema, UncoveredToApSchema::is_valid(&instance), &instance);
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"string"}},"patternProperties":{"^x-":{"type":"string"}},"additionalProperties":false}"#
)]
struct UncoveredToApFalse;

#[test_case(serde_json::json!({"a":"s","x-1":"t"}) ; "all_covered_ok")]
#[test_case(serde_json::json!({"a":"s","x-1":"t","other":5}) ; "extra_key_rejected")]
#[test_case(serde_json::json!({"x-1":"t"}) ; "pattern_only_ok")]
fn single_pass_uncovered_ap_false(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"string"}},"patternProperties":{"^x-":{"type":"string"}},"additionalProperties":false});
    assert_is_valid_parity(&schema, UncoveredToApFalse::is_valid(&instance), &instance);
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"string"}},"patternProperties":{"^x-":{"type":"string"}},"required":["a","b"]}"#
)]
struct RequiredMix;

#[test_case(serde_json::json!({"a":"s","b":1}) ; "both_required_present")]
#[test_case(serde_json::json!({"a":"s"}) ; "required_only_missing")]
#[test_case(serde_json::json!({"b":1}) ; "required_in_props_missing")]
#[test_case(serde_json::json!({"a":"s","b":1,"x-1":"t"}) ; "with_pattern_key")]
fn single_pass_required_mix(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"string"}},"patternProperties":{"^x-":{"type":"string"}},"required":["a","b"]});
    assert_is_valid_parity(&schema, RequiredMix::is_valid(&instance), &instance);
}

// items as an empty (always-valid) schema.
#[jsonschema::validator(schema = r#"{"type":"array","items":{}}"#)]
struct TrivialItems;

#[test_case(serde_json::json!([1, "x", true]) ; "any_items")]
#[test_case(serde_json::json!("not_array") ; "non_array")]
fn trivial_items_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"array","items":{}});
    assert_is_valid_parity(&schema, TrivialItems::is_valid(&instance), &instance);
}

// Draft 7 tuple items with an empty additionalItems schema.
#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"array","items":[{"type":"string"}],"additionalItems":{}}"#
)]
struct TrivialAdditionalItems;

#[test_case(serde_json::json!(["s", 1, true]) ; "extra_items_any")]
#[test_case(serde_json::json!([5]) ; "prefix_type_fails")]
fn trivial_additional_items_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"http://json-schema.org/draft-07/schema#","type":"array","items":[{"type":"string"}],"additionalItems":{}});
    assert_is_valid_parity(
        &schema,
        TrivialAdditionalItems::is_valid(&instance),
        &instance,
    );
}

// Draft 4 ignores content keywords (no content-validation vocabulary).
#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-04/schema#","contentEncoding":"base64"}"#
)]
struct Draft4ContentIgnored;

#[test_case(serde_json::json!("not base64!!") ; "any_string_ok")]
#[test_case(serde_json::json!(5) ; "non_string_ok")]
fn draft4_content_ignored_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"http://json-schema.org/draft-04/schema#","contentEncoding":"base64"});
    assert_is_valid_parity(
        &schema,
        Draft4ContentIgnored::is_valid(&instance),
        &instance,
    );
}

// properties alongside an empty (always-valid) additionalProperties schema.
#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"string"}},"additionalProperties":{}}"#
)]
struct PropertiesTrivialAdditional;

#[test_case(serde_json::json!({"a":"s"}) ; "known_ok")]
#[test_case(serde_json::json!({"a":5}) ; "known_bad")]
#[test_case(serde_json::json!({"extra":1}) ; "additional_allowed")]
fn properties_trivial_additional_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"string"}},"additionalProperties":{}});
    assert_is_valid_parity(
        &schema,
        PropertiesTrivialAdditional::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{},"unevaluatedProperties":false}"#
)]
struct UnevalPropsEmptyProperties;

#[test_case(serde_json::json!({}) ; "empty_ok")]
#[test_case(serde_json::json!({"a":1}) ; "any_key_unevaluated")]
fn uneval_props_empty_properties_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{},"unevaluatedProperties":false});
    assert_is_valid_parity(
        &schema,
        UnevalPropsEmptyProperties::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","allOf":[{"properties":{"a":{}},"unevaluatedProperties":{"type":"string"}}],"unevaluatedProperties":false}"#
)]
struct NestedUnevalPropsSchema;

#[test_case(serde_json::json!({}) ; "empty_ok")]
#[test_case(serde_json::json!({"a":1,"b":"s"}) ; "extra_string_ok")]
#[test_case(serde_json::json!({"a":1,"b":2}) ; "extra_non_string_bad")]
fn nested_uneval_props_schema_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","allOf":[{"properties":{"a":{}},"unevaluatedProperties":{"type":"string"}}],"unevaluatedProperties":false});
    assert_is_valid_parity(
        &schema,
        NestedUnevalPropsSchema::is_valid(&instance),
        &instance,
    );
}

// Fractional minimum bound.
#[jsonschema::validator(schema = r#"{"type":"number","minimum":1.5}"#)]
struct FractionalMinimum;

#[test_case(serde_json::json!(2) ; "above")]
#[test_case(serde_json::json!(1.5) ; "equal")]
#[test_case(serde_json::json!(1) ; "below")]
fn fractional_minimum_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"number","minimum":1.5});
    assert_is_valid_parity(&schema, FractionalMinimum::is_valid(&instance), &instance);
}

// oneOf distinguished by a string const on a shared key.
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["kind"],"properties":{"kind":{"const":"a"},"v":{"type":"string"}}},{"type":"object","required":["kind"],"properties":{"kind":{"const":"b"},"v":{"type":"integer"}}}]}"#
)]
struct StringDiscriminator;

#[test_case(serde_json::json!({"kind":"a","v":"s"}) ; "a_ok")]
#[test_case(serde_json::json!({"kind":"a","v":5}) ; "a_bad")]
#[test_case(serde_json::json!({"kind":"b","v":5}) ; "b_ok")]
#[test_case(serde_json::json!({"kind":"c"}) ; "no_branch")]
fn string_discriminator_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["kind"],"properties":{"kind":{"const":"a"},"v":{"type":"string"}}},{"type":"object","required":["kind"],"properties":{"kind":{"const":"b"},"v":{"type":"integer"}}}]});
    assert_is_valid_parity(&schema, StringDiscriminator::is_valid(&instance), &instance);
}

// oneOf distinguished by a boolean const on a shared key.
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","properties":{"flag":{"const":true},"v":{"type":"string"}},"required":["flag"]},{"type":"object","properties":{"flag":{"const":false},"v":{"type":"integer"}},"required":["flag"]}]}"#
)]
struct BoolDiscriminator;

#[test_case(serde_json::json!({"flag":true,"v":"s"}) ; "true_branch_ok")]
#[test_case(serde_json::json!({"flag":true,"v":5}) ; "true_branch_bad")]
#[test_case(serde_json::json!({"flag":false,"v":5}) ; "false_branch_ok")]
#[test_case(serde_json::json!({"flag":false,"v":"s"}) ; "false_branch_bad")]
fn bool_discriminator_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","properties":{"flag":{"const":true},"v":{"type":"string"}},"required":["flag"]},{"type":"object","properties":{"flag":{"const":false},"v":{"type":"integer"}},"required":["flag"]}]});
    assert_is_valid_parity(&schema, BoolDiscriminator::is_valid(&instance), &instance);
}

// additionalProperties:false with an exact-anchored pattern.
#[jsonschema::validator(
    schema = r#"{"type":"object","additionalProperties":false,"patternProperties":{"^foo$":{"type":"string"}}}"#
)]
struct ExactPatternCoverage;

#[test_case(serde_json::json!({"foo":"s"}) ; "exact_key_ok")]
#[test_case(serde_json::json!({"foo":5}) ; "exact_key_bad_type")]
#[test_case(serde_json::json!({"other":1}) ; "extra_key_rejected")]
#[test_case(serde_json::json!({}) ; "empty")]
fn exact_pattern_coverage_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","additionalProperties":false,"patternProperties":{"^foo$":{"type":"string"}}});
    assert_is_valid_parity(
        &schema,
        ExactPatternCoverage::is_valid(&instance),
        &instance,
    );
}

// additionalProperties schema alongside both a literal-anchored and an
// unanchored pattern.
#[jsonschema::validator(
    schema = r#"{"type":"object","additionalProperties":{"type":"string"},"patternProperties":{"^foo$":{"type":"integer"},"bar":{"type":"boolean"}}}"#
)]
struct AdditionalSchemaWithLiteralAndRegexPatterns;

#[test_case(serde_json::json!({"foo": 1}) ; "literal_pattern_ok")]
#[test_case(serde_json::json!({"foo": "x"}) ; "literal_pattern_bad")]
#[test_case(serde_json::json!({"xbary": true}) ; "regex_pattern_ok")]
#[test_case(serde_json::json!({"xbary": 1}) ; "regex_pattern_bad")]
#[test_case(serde_json::json!({"other": "s"}) ; "additional_ok")]
#[test_case(serde_json::json!({"other": 1}) ; "additional_bad")]
fn test_additional_schema_with_literal_and_regex_patterns(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","additionalProperties":{"type":"string"},"patternProperties":{"^foo$":{"type":"integer"},"bar":{"type":"boolean"}}});
    assert_is_valid_parity(
        &schema,
        AdditionalSchemaWithLiteralAndRegexPatterns::is_valid(&instance),
        &instance,
    );
}

// type union of string and number with no number-specific keywords.
#[jsonschema::validator(schema = r#"{"type":["string","number"]}"#)]
struct StringOrNumber;

#[test_case(serde_json::json!(1.5) ; "number")]
#[test_case(serde_json::json!("s") ; "string")]
#[test_case(serde_json::json!(true) ; "bool_rejected")]
fn string_or_number_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":["string","number"]});
    assert_is_valid_parity(&schema, StringOrNumber::is_valid(&instance), &instance);
}

// oneOf over const/enum literal shapes on a shared key.
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":null}}},{"type":"object","required":["k"],"properties":{"k":{"enum":[true,false]}}},{"type":"object","required":["k"],"properties":{"k":{"enum":[null]}}},{"type":"object","required":["k"],"properties":{"k":{"enum":[]}}},{"type":"object","required":["x"],"properties":{}}]}"#
)]
struct OneOfDiscriminatorEdges;

#[test_case(serde_json::json!({"k":true}) ; "enum_bool")]
#[test_case(serde_json::json!({"x":1}) ; "required_missing_branch")]
#[test_case(serde_json::json!({}) ; "empty")]
fn one_of_discriminator_edges_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":null}}},{"type":"object","required":["k"],"properties":{"k":{"enum":[true,false]}}},{"type":"object","required":["k"],"properties":{"k":{"enum":[null]}}},{"type":"object","required":["k"],"properties":{"k":{"enum":[]}}},{"type":"object","required":["x"],"properties":{}}]});
    assert_is_valid_parity(
        &schema,
        OneOfDiscriminatorEdges::is_valid(&instance),
        &instance,
    );
}

// oneOf where one branch lacks the key the others share.
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["a"],"properties":{"a":{"const":"a1"}}},{"type":"object","required":["a"],"properties":{"a":{"const":"a2"}}},{"type":"object","required":["b"],"properties":{"b":{"const":"b1"}}}]}"#
)]
struct OneOfBranchMissingKey;

#[test_case(serde_json::json!({"a":"a1"}) ; "keyed_branch")]
#[test_case(serde_json::json!({"b":"b1"}) ; "unkeyed_branch")]
fn one_of_branch_missing_key_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["a"],"properties":{"a":{"const":"a1"}}},{"type":"object","required":["a"],"properties":{"a":{"const":"a2"}}},{"type":"object","required":["b"],"properties":{"b":{"const":"b1"}}}]});
    assert_is_valid_parity(
        &schema,
        OneOfBranchMissingKey::is_valid(&instance),
        &instance,
    );
}

// oneOf with two candidate keys distinguishing the branches.
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["a","b"],"properties":{"a":{"const":"a1"},"b":{"const":"b1"}}},{"type":"object","required":["a","b"],"properties":{"a":{"const":"a2"},"b":{"const":"b2"}}},{"type":"object","required":["b"],"properties":{"b":{"const":"b3"}}}]}"#
)]
struct OneOfDiscriminatorTiebreak;

#[test_case(serde_json::json!({"a":"a1","b":"b1"}) ; "first")]
#[test_case(serde_json::json!({"b":"b3"}) ; "third")]
fn one_of_discriminator_tiebreak_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["a","b"],"properties":{"a":{"const":"a1"},"b":{"const":"b1"}}},{"type":"object","required":["a","b"],"properties":{"a":{"const":"a2"},"b":{"const":"b2"}}},{"type":"object","required":["b"],"properties":{"b":{"const":"b3"}}}]});
    assert_is_valid_parity(
        &schema,
        OneOfDiscriminatorTiebreak::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["type"],"properties":{"type":{"type":"string","enum":["Point"]},"c":{"type":"number"}}},{"type":"object","required":["type"],"properties":{"type":{"type":"string","enum":["LineString","MultiLineString"]},"c":{"type":"string"}}}]}"#
)]
struct OneOfEnumTypedDiscriminator;

#[test_case(serde_json::json!({"type":"Point","c":1}) ; "first_branch")]
#[test_case(serde_json::json!({"type":"MultiLineString","c":"x"}) ; "second_branch_second_literal")]
#[test_case(serde_json::json!({"type":"Point","c":"x"}) ; "matched_tag_body_mismatch")]
#[test_case(serde_json::json!({"type":5}) ; "non_string_tag")]
#[test_case(serde_json::json!({"c":1}) ; "missing_tag")]
fn one_of_enum_typed_discriminator_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["type"],"properties":{"type":{"type":"string","enum":["Point"]},"c":{"type":"number"}}},{"type":"object","required":["type"],"properties":{"type":{"type":"string","enum":["LineString","MultiLineString"]},"c":{"type":"string"}}}]});
    assert_is_valid_parity(
        &schema,
        OneOfEnumTypedDiscriminator::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        OneOfEnumTypedDiscriminator::is_valid(&instance),
        OneOfEnumTypedDiscriminator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["tag"],"additionalProperties":false,"properties":{"tag":{"const":"br"}}},{"type":"object","required":["tag"],"additionalProperties":false,"properties":{"tag":{"const":"p"},"data":{"type":"string"}}}]}"#
)]
struct OneOfDiscriminatorAdditionalFalse;

#[test_case(serde_json::json!({"tag":"br"}) ; "tag_only_branch")]
#[test_case(serde_json::json!({"tag":"br","extra":1}) ; "unexpected_key")]
#[test_case(serde_json::json!({"tag":"p","data":"x"}) ; "second_branch")]
#[test_case(serde_json::json!({"tag":"p","data":5}) ; "second_branch_bad_value")]
fn one_of_discriminator_additional_false_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["tag"],"additionalProperties":false,"properties":{"tag":{"const":"br"}}},{"type":"object","required":["tag"],"additionalProperties":false,"properties":{"tag":{"const":"p"},"data":{"type":"string"}}}]});
    assert_is_valid_parity(
        &schema,
        OneOfDiscriminatorAdditionalFalse::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        OneOfDiscriminatorAdditionalFalse::is_valid(&instance),
        OneOfDiscriminatorAdditionalFalse::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":"a","enum":["b"]}}},{"type":"object","required":["k"],"properties":{"k":{"const":"abcdef","minLength":3}}}]}"#
)]
struct OneOfDiscriminatorSiblingConstraints;

#[test_case(serde_json::json!({"k":"a"}) ; "const_passes_enum_sibling_fails")]
#[test_case(serde_json::json!({"k":"abcdef"}) ; "const_and_sibling_pass")]
#[test_case(serde_json::json!({"k":"b"}) ; "no_tag_match")]
fn one_of_discriminator_sibling_constraints_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":"a","enum":["b"]}}},{"type":"object","required":["k"],"properties":{"k":{"const":"abcdef","minLength":3}}}]});
    assert_is_valid_parity(
        &schema,
        OneOfDiscriminatorSiblingConstraints::is_valid(&instance),
        &instance,
    );
}

// patternProperties with an empty (always-valid) additionalProperties schema.
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":"a"},"x":{"type":"string"}}},{"type":"object","required":["k"],"properties":{"k":{"const":"b"},"x":{"type":"number"}}}]}"#,
    draft = referencing::Draft::Draft4
)]
struct OneOfConstTagsDraft4;

#[test_case(serde_json::json!({"k":"zzz","x":"s"}) ; "const_inert_first_branch_valid")]
#[test_case(serde_json::json!({"k":"a","x":"s"}) ; "tag_and_body_valid")]
#[test_case(serde_json::json!({"k":"a","x":1}) ; "second_body_valid_despite_first_tag")]
#[test_case(serde_json::json!({"x":"s"}) ; "missing_tag")]
fn one_of_const_tags_draft4_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":"a"},"x":{"type":"string"}}},{"type":"object","required":["k"],"properties":{"k":{"const":"b"},"x":{"type":"number"}}}]});
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft4)
        .build(&schema)
        .expect("valid schema");
    let expected = runtime.is_valid(&instance);
    assert_eq!(OneOfConstTagsDraft4::is_valid(&instance), expected);
    assert_eq!(OneOfConstTagsDraft4::validate(&instance).is_ok(), expected);
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"enum":["a","b"]},"x":{"type":"string"}}},{"type":"object","required":["k"],"properties":{"k":{"enum":["b","c"]},"x":{"type":"number"}}}]}"#
)]
struct OneOfOverlappingEnumTags;

#[test_case(serde_json::json!({"k":"a","x":"s"}) ; "unshared_tag")]
#[test_case(serde_json::json!({"k":"b","x":"s"}) ; "shared_tag_first_body")]
#[test_case(serde_json::json!({"k":"b","x":1}) ; "shared_tag_second_body")]
#[test_case(serde_json::json!({"k":"b"}) ; "shared_tag_both_bodies")]
#[test_case(serde_json::json!({"k":"z"}) ; "unknown_tag")]
fn one_of_overlapping_enum_tags_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"enum":["a","b"]},"x":{"type":"string"}}},{"type":"object","required":["k"],"properties":{"k":{"enum":["b","c"]},"x":{"type":"number"}}}]});
    assert_is_valid_parity(
        &schema,
        OneOfOverlappingEnumTags::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        OneOfOverlappingEnumTags::is_valid(&instance),
        OneOfOverlappingEnumTags::validate(&instance),
        &instance,
    );
}

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":9007199254740993},"x":{"type":"string"}}},{"type":"object","required":["k"],"properties":{"k":{"const":2},"x":{"type":"number"}}}]}"#
)]
struct OneOfHugeIntTags;

#[cfg(feature = "arbitrary-precision")]
#[test_case(r#"{"k":9007199254740993,"x":"s"}"# ; "integer_form")]
#[test_case(r#"{"k":9007199254740993.0,"x":"s"}"# ; "float_form_exact_under_arbitrary_precision")]
#[test_case(r#"{"k":9007199254740992.0,"x":"s"}"# ; "adjacent_float")]
#[test_case(r#"{"k":2,"x":1}"# ; "small_tag")]
fn one_of_huge_int_tags_matches_runtime(instance_json: &str) {
    let schema = serde_json::json!({"oneOf":[{"type":"object","required":["k"],"properties":{"k":{"const":9_007_199_254_740_993_i64},"x":{"type":"string"}}},{"type":"object","required":["k"],"properties":{"k":{"const":2},"x":{"type":"number"}}}]});
    let instance: serde_json::Value =
        serde_json::from_str(instance_json).expect("valid instance json");
    let expected = runtime_valid(&schema, &instance);
    assert_eq!(OneOfHugeIntTags::is_valid(&instance), expected);
    assert_eq!(OneOfHugeIntTags::validate(&instance).is_ok(), expected);
}

#[jsonschema::validator(
    schema = r#"{"type":"object","patternProperties":{"^x-":{"type":"string"}},"additionalProperties":{}}"#
)]
struct PatternTrivialAp;

#[test_case(serde_json::json!({"x-1":"t"}) ; "pattern_ok")]
#[test_case(serde_json::json!({"x-1":5}) ; "pattern_fails")]
#[test_case(serde_json::json!({"other":123}) ; "uncovered_trivial_ap")]
fn single_pass_pattern_trivial_ap(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","patternProperties":{"^x-":{"type":"string"}},"additionalProperties":{}});
    assert_is_valid_parity(&schema, PatternTrivialAp::is_valid(&instance), &instance);
}

// empty patternProperties object.
#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"a":{"type":"string"}},"patternProperties":{}}"#
)]
struct EmptyPatternProps;

#[test_case(serde_json::json!({"a":"s"}) ; "prop_ok")]
#[test_case(serde_json::json!({"a":5}) ; "prop_fails")]
#[test_case(serde_json::json!({"other":1}) ; "extra_key_allowed")]
fn single_pass_empty_pattern_props(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{"a":{"type":"string"}},"patternProperties":{}});
    assert_is_valid_parity(&schema, EmptyPatternProps::is_valid(&instance), &instance);
}

#[jsonschema::validator(
    schema = r##"{"definitions":{"pos":{"type":"integer","minimum":1}},"type":"object","properties":{"id":{"$ref":"#/definitions/pos"}},"required":["id"]}"##,
    draft = Draft7
)]
struct RefDefinitionSchemaPath;

#[test]
fn test_ref_error_reports_definition_site_schema_path() {
    let schema = serde_json::json!({"definitions":{"pos":{"type":"integer","minimum":1}},"type":"object","properties":{"id":{"$ref":"#/definitions/pos"}},"required":["id"]});
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&schema)
        .expect("valid schema");
    let instance = serde_json::json!({"id":0});
    let generated = RefDefinitionSchemaPath::validate(&instance).expect_err("invalid");
    let expected = runtime.validate(&instance).expect_err("invalid");
    assert_eq!(
        generated.schema_path().as_str(),
        expected.schema_path().as_str(),
        "schema_path"
    );
    assert_eq!(
        generated.instance_path().as_str(),
        expected.instance_path().as_str(),
        "instance_path"
    );
}

// With the validation vocabulary disabled, every validation keyword family
// (numeric/string/array/object) must be dropped by codegen, matching the runtime.
#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-validation","type":["number","string","array","object"],"minimum":5,"minLength":2,"minItems":1,"required":["x"],"multipleOf":2}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct NoValidationVocabAllFamilies;

#[test_case(serde_json::json!(3) ; "number")]
#[test_case(serde_json::json!("a") ; "string")]
#[test_case(serde_json::json!([]) ; "array")]
#[test_case(serde_json::json!({}) ; "object")]
fn test_no_validation_vocabulary_drops_all_keyword_families(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"json-schema:///meta/no-validation","type":["number","string","array","object"],"minimum":5,"minLength":2,"minItems":1,"required":["x"],"multipleOf":2});
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-validation",
            serde_json::json!({"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}),
        )],
    );
    // Validation vocab off -> no constraints enforced -> everything valid, codegen == runtime.
    assert!(NoValidationVocabAllFamilies::is_valid(&instance));
    assert_eq!(
        NoValidationVocabAllFamilies::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-validation","type":"array","contains":{},"minItems":5}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct NoValidationVocabArrayApplicator;

#[test_case(serde_json::json!([1]) ; "below_dropped_min_items")]
#[test_case(serde_json::json!([]) ; "contains_unsatisfied")]
#[test_case(serde_json::json!([1, 2]) ; "contains_satisfied")]
fn test_no_validation_vocab_array_applicator(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"json-schema:///meta/no-validation","type":"array","contains":{},"minItems":5});
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-validation",
            serde_json::json!({"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}),
        )],
    );
    assert_eq!(
        NoValidationVocabArrayApplicator::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

// oneOf branches that are `$ref`s must be resolved for discriminator analysis
// (exercises the top-level `$ref` hop in refs.rs).
#[jsonschema::validator(
    schema = r##"{"$defs":{"a":{"type":"object","properties":{"kind":{"const":"a"},"x":{"type":"integer"}},"required":["kind","x"]},"b":{"type":"object","properties":{"kind":{"const":"b"},"y":{"type":"string"}},"required":["kind","y"]}},"oneOf":[{"$ref":"#/$defs/a"},{"$ref":"#/$defs/b"}]}"##,
    draft = Draft202012
)]
struct OneOfRefBranches;

#[test_case(serde_json::json!({"kind":"a","x":1}) ; "branch_a")]
#[test_case(serde_json::json!({"kind":"b","y":"s"}) ; "branch_b")]
#[test_case(serde_json::json!({"kind":"a","x":"nope"}) ; "branch_a_bad")]
#[test_case(serde_json::json!({"kind":"c"}) ; "no_branch")]
fn test_one_of_ref_branches_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$defs":{"a":{"type":"object","properties":{"kind":{"const":"a"},"x":{"type":"integer"}},"required":["kind","x"]},"b":{"type":"object","properties":{"kind":{"const":"b"},"y":{"type":"string"}},"required":["kind","y"]}},"oneOf":[{"$ref":"#/$defs/a"},{"$ref":"#/$defs/b"}]});
    assert_is_valid_parity(&schema, OneOfRefBranches::is_valid(&instance), &instance);
}

// A `$ref` with an empty fragment (trailing `#`) resolves to the resource root.
#[jsonschema::validator(
    schema = r##"{"$ref":"json-schema:///thing#"}"##,
    draft = Draft202012,
    resources = { "json-schema:///thing" => { schema = r#"{"type":"integer"}"# } }
)]
struct EmptyFragmentRef;

#[test_case(serde_json::json!(1) ; "integer_ok")]
#[test_case(serde_json::json!("x") ; "string_bad")]
fn test_empty_fragment_ref_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$ref":"json-schema:///thing#"});
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///thing",
            serde_json::json!({"type":"integer"}),
        )],
    );
    assert_eq!(
        EmptyFragmentRef::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"b":true},"type":"object","allOf":[{"$ref":"#/$defs/b"}],"unevaluatedProperties":false}"##,
    draft = Draft202012
)]
struct UnevalPropsRefBoolTarget;

#[test_case(serde_json::json!({}) ; "empty")]
#[test_case(serde_json::json!({"x":1}) ; "unevaluated_property")]
fn test_uneval_props_ref_bool_target(instance: serde_json::Value) {
    let schema = serde_json::json!({"$defs":{"b":true},"type":"object","allOf":[{"$ref":"#/$defs/b"}],"unevaluatedProperties":false});
    assert_validate_parity_for(
        &schema,
        UnevalPropsRefBoolTarget::is_valid(&instance),
        UnevalPropsRefBoolTarget::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"b":true},"type":"array","allOf":[{"$ref":"#/$defs/b"}],"unevaluatedItems":false}"##,
    draft = Draft202012
)]
struct UnevalItemsRefBoolTarget;

#[test_case(serde_json::json!([]) ; "empty")]
#[test_case(serde_json::json!([1]) ; "unevaluated_item")]
fn test_uneval_items_ref_bool_target(instance: serde_json::Value) {
    let schema = serde_json::json!({"$defs":{"b":true},"type":"array","allOf":[{"$ref":"#/$defs/b"}],"unevaluatedItems":false});
    assert_validate_parity_for(
        &schema,
        UnevalItemsRefBoolTarget::is_valid(&instance),
        UnevalItemsRefBoolTarget::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(schema = r#"{"type":"object","properties":{},"additionalProperties":false}"#, draft = Draft202012)]
struct EmptyPropertiesAdditionalFalse;

#[test_case(serde_json::json!({}) ; "empty")]
#[test_case(serde_json::json!({"a":1}) ; "extra_key")]
fn test_empty_properties_additional_false(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{},"additionalProperties":false});
    assert_validate_parity_for(
        &schema,
        EmptyPropertiesAdditionalFalse::is_valid(&instance),
        EmptyPropertiesAdditionalFalse::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","anyOf":[{"type":"object","allOf":[{"properties":{"a":{"type":"integer"}}}]}],"unevaluatedProperties":false}"#,
    draft = Draft202012
)]
struct UnevalGuardBranchWithSiblingKeyword;

#[test_case(serde_json::json!({"a":1}) ; "evaluated")]
#[test_case(serde_json::json!({"a":1,"b":2}) ; "extra_unevaluated")]
#[test_case(serde_json::json!({"a":"x"}) ; "branch_fails")]
fn test_uneval_guard_branch_with_sibling_keyword(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","anyOf":[{"type":"object","allOf":[{"properties":{"a":{"type":"integer"}}}]}],"unevaluatedProperties":false});
    assert_validate_parity_for(
        &schema,
        UnevalGuardBranchWithSiblingKeyword::is_valid(&instance),
        UnevalGuardBranchWithSiblingKeyword::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{},"additionalProperties":{"type":"string"},"unevaluatedProperties":false}"#,
    draft = Draft202012
)]
struct UnevalEmptyProperties;

#[test_case(serde_json::json!({}) ; "empty")]
#[test_case(serde_json::json!({"x":"s"}) ; "additional_covers")]
#[test_case(serde_json::json!({"x":1}) ; "additional_fails")]
fn test_uneval_empty_properties(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","properties":{},"additionalProperties":{"type":"string"},"unevaluatedProperties":false});
    assert_validate_parity_for(
        &schema,
        UnevalEmptyProperties::is_valid(&instance),
        UnevalEmptyProperties::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-applicator","type":"object","unevaluatedProperties":false}"#,
    draft = Draft202012,
    resources = {
        "json-schema:///meta/no-applicator" => { schema = r#"{"$id":"json-schema:///meta/no-applicator","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct UnevalNoApplicatorVocabProps;

#[test_case(serde_json::json!({}) ; "empty")]
#[test_case(serde_json::json!({"a":1}) ; "any_property_unevaluated")]
fn test_uneval_no_applicator_vocab_props(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"json-schema:///meta/no-applicator","type":"object","unevaluatedProperties":false});
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-applicator",
            serde_json::json!({"$id":"json-schema:///meta/no-applicator","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}),
        )],
    );
    assert_eq!(
        UnevalNoApplicatorVocabProps::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-applicator","type":"array","unevaluatedItems":false}"#,
    draft = Draft202012,
    resources = {
        "json-schema:///meta/no-applicator" => { schema = r#"{"$id":"json-schema:///meta/no-applicator","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct UnevalNoApplicatorVocabItems;

#[test_case(serde_json::json!([]) ; "empty")]
#[test_case(serde_json::json!([1]) ; "any_item_unevaluated")]
fn test_uneval_no_applicator_vocab_items(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"json-schema:///meta/no-applicator","type":"array","unevaluatedItems":false});
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-applicator",
            serde_json::json!({"$id":"json-schema:///meta/no-applicator","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}),
        )],
    );
    assert_eq!(
        UnevalNoApplicatorVocabItems::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(
    schema = r##"{"$schema":"json-schema:///meta/no-unevaluated","$id":"json-schema:///tree-no-uneval","$dynamicAnchor":"node","type":"object","properties":{"children":{"type":"array","items":{"$dynamicRef":"#node"}}}}"##,
    draft = Draft202012,
    resources = {
        "json-schema:///meta/no-unevaluated" => { schema = r#"{"$id":"json-schema:///meta/no-unevaluated","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct DynamicRefNoUnevaluatedVocab;

#[test_case(serde_json::json!({}) ; "empty")]
#[test_case(serde_json::json!({"children":[{}]}) ; "nested")]
#[test_case(serde_json::json!({"children":[{"children":[{}]}]}) ; "deep")]
fn test_dynamic_ref_no_unevaluated_vocab(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"json-schema:///meta/no-unevaluated","$id":"json-schema:///tree-no-uneval","$dynamicAnchor":"node","type":"object","properties":{"children":{"type":"array","items":{"$dynamicRef":"#node"}}}});
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/no-unevaluated",
            serde_json::json!({"$id":"json-schema:///meta/no-unevaluated","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}),
        )],
    );
    assert_eq!(
        DynamicRefNoUnevaluatedVocab::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(schema = r#"{"type":["number","string"],"minLength":1}"#, draft = Draft202012)]
struct MultiTypeNumberFallbackWithStringKeyword;

#[test_case(serde_json::json!(3.5) ; "number")]
#[test_case(serde_json::json!("hi") ; "string_ok")]
#[test_case(serde_json::json!("") ; "string_too_short")]
fn test_multi_type_number_fallback(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":["number","string"],"minLength":1});
    assert_validate_parity_for(
        &schema,
        MultiTypeNumberFallbackWithStringKeyword::is_valid(&instance),
        MultiTypeNumberFallbackWithStringKeyword::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(schema = r#"{"type":42,"minLength":1}"#, draft = Draft202012)]
struct TypeNonStringArrayValue;

#[test_case(serde_json::json!(5) ; "number_rejected")]
#[test_case(serde_json::json!(true) ; "bool_rejected")]
fn test_type_non_string_array_value(instance: serde_json::Value) {
    // A `type` whose value is neither a string nor an array is malformed; codegen emits a
    // reject-everything validator rather than a compile error.
    assert!(!TypeNonStringArrayValue::is_valid(&instance));
    assert!(TypeNonStringArrayValue::validate(&instance).is_err());
}

#[jsonschema::validator(
    schema = r##"{"$schema":"http://json-schema.org/draft-06/schema#","definitions":{"a":{"properties":{"resourceType":{"const":"A"},"x":{"type":"integer"}},"required":["resourceType"],"additionalProperties":false},"b":{"properties":{"resourceType":{"const":"B"},"y":{"type":"string"}},"required":["resourceType"],"additionalProperties":false}},"oneOf":[{"$ref":"#/definitions/a"},{"$ref":"#/definitions/b"}]}"##
)]
struct OneOfVacuousRefBranches;

#[test_case(serde_json::json!({"resourceType":"A","x":1}) ; "branch_a")]
#[test_case(serde_json::json!({"resourceType":"B","y":"s"}) ; "branch_b")]
#[test_case(serde_json::json!({"resourceType":"A","x":"bad"}) ; "branch_a_body_mismatch")]
#[test_case(serde_json::json!({"resourceType":"C"}) ; "unknown_tag")]
#[test_case(serde_json::json!({}) ; "missing_tag")]
#[test_case(serde_json::json!("text") ; "string_matches_both_vacuously")]
#[test_case(serde_json::json!(5) ; "number_matches_both_vacuously")]
#[test_case(serde_json::json!([1]) ; "array_matches_both_vacuously")]
fn test_one_of_vacuous_ref_branches(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema":"http://json-schema.org/draft-06/schema#","definitions":{"a":{"properties":{"resourceType":{"const":"A"},"x":{"type":"integer"}},"required":["resourceType"],"additionalProperties":false},"b":{"properties":{"resourceType":{"const":"B"},"y":{"type":"string"}},"required":["resourceType"],"additionalProperties":false}},"oneOf":[{"$ref":"#/definitions/a"},{"$ref":"#/definitions/b"}]});
    assert_is_valid_parity(
        &schema,
        OneOfVacuousRefBranches::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        OneOfVacuousRefBranches::is_valid(&instance),
        OneOfVacuousRefBranches::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"properties":{"k":{"const":"a"},"x":{"type":"integer"}},"required":["k"],"additionalProperties":false},{"properties":{"k":{"const":"b"}},"required":["k"],"additionalProperties":false}]}"#
)]
struct OneOfVacuousInlineBranches;

#[test_case(serde_json::json!({"k":"a","x":1}) ; "branch_a")]
#[test_case(serde_json::json!({"k":"a","x":"bad"}) ; "branch_a_body_mismatch")]
#[test_case(serde_json::json!({"k":"b"}) ; "branch_b")]
#[test_case(serde_json::json!({"k":"b","extra":1}) ; "additional_property_rejected")]
#[test_case(serde_json::json!("text") ; "string_matches_both_vacuously")]
#[test_case(serde_json::json!(null) ; "null_matches_both_vacuously")]
fn test_one_of_vacuous_inline_branches(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"properties":{"k":{"const":"a"},"x":{"type":"integer"}},"required":["k"],"additionalProperties":false},{"properties":{"k":{"const":"b"}},"required":["k"],"additionalProperties":false}]});
    assert_is_valid_parity(
        &schema,
        OneOfVacuousInlineBranches::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        OneOfVacuousInlineBranches::is_valid(&instance),
        OneOfVacuousInlineBranches::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"type":"object","properties":{"k":{"const":"b"}},"required":["k"]}]}"#
)]
struct OneOfSingleVacuousBranch;

#[test_case(serde_json::json!("text") ; "string_valid_via_vacuous_branch")]
#[test_case(serde_json::json!(7) ; "number_valid_via_vacuous_branch")]
#[test_case(serde_json::json!({"k":"a"}) ; "branch_a")]
#[test_case(serde_json::json!({"k":"b"}) ; "branch_b")]
#[test_case(serde_json::json!({}) ; "missing_tag")]
fn test_one_of_single_vacuous_branch(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"type":"object","properties":{"k":{"const":"b"}},"required":["k"]}]});
    assert_is_valid_parity(
        &schema,
        OneOfSingleVacuousBranch::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"properties":{"k":{"const":"b"}},"required":["k"]},{"type":"string"}]}"#
)]
struct OneOfVacuousWithStringBranch;

#[test_case(serde_json::json!("text") ; "string_matches_three_branches")]
#[test_case(serde_json::json!(7) ; "number_matches_two_branches")]
#[test_case(serde_json::json!({"k":"a"}) ; "branch_a")]
fn test_one_of_vacuous_with_string_branch(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"properties":{"k":{"const":"b"}},"required":["k"]},{"type":"string"}]});
    assert_is_valid_parity(
        &schema,
        OneOfVacuousWithStringBranch::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"properties":{"k":{"const":"b"}},"required":["k"]},{"properties":{"z":{"type":"integer"}}}]}"#
)]
struct OneOfVacuousUncoveredBranch;

struct StringGateKeyword {
    allow_strings: bool,
}

impl jsonschema::Keyword for StringGateKeyword {
    fn validate<'i>(
        &self,
        instance: &'i serde_json::Value,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(jsonschema::ValidationError::custom("strings rejected"))
        }
    }

    fn is_valid(&self, instance: &serde_json::Value) -> bool {
        !instance.is_string() || self.allow_strings
    }
}

// The Result wrapping is required by the keyword factory signature.
#[allow(clippy::unnecessary_wraps)]
fn string_gate_factory<'a>(
    _parent: &'a serde_json::Map<String, serde_json::Value>,
    value: &'a serde_json::Value,
    _path: jsonschema::paths::Location,
) -> Result<Box<dyn jsonschema::Keyword>, jsonschema::ValidationError<'a>> {
    Ok(Box::new(StringGateKeyword {
        allow_strings: value.as_str() == Some("strings-ok"),
    }))
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"],"description":"strings-ok"},{"properties":{"k":{"const":"b"}},"required":["k"],"description":"strings-bad"}]}"#,
    keywords = { "description" => crate::string_gate_factory }
)]
struct OneOfVacuousWithCustomKeyword;

#[test_case(serde_json::json!("text") ; "string_valid_via_custom_keyword_gate")]
#[test_case(serde_json::json!(7) ; "number_matches_both_vacuously")]
#[test_case(serde_json::json!({"k":"a"}) ; "branch_a")]
#[test_case(serde_json::json!({"k":"b"}) ; "branch_b")]
fn test_one_of_vacuous_with_custom_keyword(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"],"description":"strings-ok"},{"properties":{"k":{"const":"b"}},"required":["k"],"description":"strings-bad"}]});
    let runtime = jsonschema::options()
        .with_keyword("description", string_gate_factory)
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        OneOfVacuousWithCustomKeyword::is_valid(&instance),
        runtime.is_valid(&instance),
        "codegen/runtime mismatch for {instance}"
    );
}

#[test_case(serde_json::json!(7) ; "number_matches_three_branches")]
#[test_case(serde_json::json!({"k":"a"}) ; "branch_a_also_matches_uncovered")]
#[test_case(serde_json::json!({"z":1}) ; "uncovered_branch_only")]
fn test_one_of_vacuous_uncovered_branch(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"properties":{"k":{"const":"a"}},"required":["k"]},{"properties":{"k":{"const":"b"}},"required":["k"]},{"properties":{"z":{"type":"integer"}}}]});
    assert_is_valid_parity(
        &schema,
        OneOfVacuousUncoveredBranch::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","required":["a","b"],"properties":{"a":{"type":"string"}}}"#
)]
struct RequiredFusedIntoProperties;

#[test_case(serde_json::json!({"a":"x","b":1}) ; "all_required_present")]
#[test_case(serde_json::json!({"a":"x"}) ; "missing_scan_only_required")]
#[test_case(serde_json::json!({"b":1}) ; "missing_property_required")]
#[test_case(serde_json::json!({"a":1,"b":1}) ; "property_check_fails")]
#[test_case(serde_json::json!({"a":"x","b":1,"extra":true}) ; "extra_key")]
#[test_case(serde_json::json!({}) ; "empty_object")]
#[test_case(serde_json::json!("text") ; "non_object")]
fn test_required_fused_into_properties(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","required":["a","b"],"properties":{"a":{"type":"string"}}});
    assert_is_valid_parity(
        &schema,
        RequiredFusedIntoProperties::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        RequiredFusedIntoProperties::is_valid(&instance),
        RequiredFusedIntoProperties::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","required":["a","b"],"properties":{"a":{"type":"string"},"b":{"type":"integer"}},"additionalProperties":false}"#
)]
struct RequiredFusedWithAdditionalFalse;

#[test_case(serde_json::json!({"a":"x","b":1}) ; "valid")]
#[test_case(serde_json::json!({"a":"x"}) ; "missing_required")]
#[test_case(serde_json::json!({"a":"x","b":1,"extra":true}) ; "additional_rejected")]
fn test_required_fused_with_additional_false(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","required":["a","b"],"properties":{"a":{"type":"string"},"b":{"type":"integer"}},"additionalProperties":false});
    assert_is_valid_parity(
        &schema,
        RequiredFusedWithAdditionalFalse::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        RequiredFusedWithAdditionalFalse::is_valid(&instance),
        RequiredFusedWithAdditionalFalse::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"type":"object","required":["a"],"additionalProperties":{"type":"string"}}"#
)]
struct RequiredFusedWithAdditionalSchema;

#[test_case(serde_json::json!({"a":"x"}) ; "valid")]
#[test_case(serde_json::json!({"b":"x"}) ; "missing_required")]
#[test_case(serde_json::json!({"a":1}) ; "additional_check_fails")]
fn test_required_fused_with_additional_schema(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","required":["a"],"additionalProperties":{"type":"string"}});
    assert_is_valid_parity(
        &schema,
        RequiredFusedWithAdditionalSchema::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"a":{"properties":{"k":{"const":"a"}},"required":["k"]},"b":{"properties":{"k":{"const":"b"}},"required":["k"]}},"oneOf":[{"$ref":"#/$defs/a","type":"string"},{"$ref":"#/$defs/b"}]}"##,
    draft = Draft202012
)]
struct OneOfRefBranchWithSibling;

#[test_case(serde_json::json!(5) ; "number_matches_bare_ref_branch_only")]
#[test_case(serde_json::json!("text") ; "string_matches_both_branches")]
#[test_case(serde_json::json!({"k":"a"}) ; "branch_a")]
#[test_case(serde_json::json!({"k":"b"}) ; "branch_b")]
fn test_one_of_ref_branch_with_sibling(instance: serde_json::Value) {
    let schema = serde_json::json!({"$defs":{"a":{"properties":{"k":{"const":"a"}},"required":["k"]},"b":{"properties":{"k":{"const":"b"}},"required":["k"]}},"oneOf":[{"$ref":"#/$defs/a","type":"string"},{"$ref":"#/$defs/b"}]});
    assert_is_valid_parity(
        &schema,
        OneOfRefBranchWithSibling::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"s":{"type":"string","format":"uri"}},"type":"object","propertyNames":{"$ref":"#/$defs/s"}}"##,
    draft = Draft202012
)]
struct PropertyNamesRefStringOnly;

#[test_case(serde_json::json!({"any key at all":1}) ; "keys_always_pass")]
#[test_case(serde_json::json!({}) ; "empty_object")]
#[test_case(serde_json::json!("text") ; "non_object")]
fn test_property_names_ref_string_only(instance: serde_json::Value) {
    let schema = serde_json::json!({"$defs":{"s":{"type":"string","format":"uri"}},"type":"object","propertyNames":{"$ref":"#/$defs/s"}});
    assert_is_valid_parity(
        &schema,
        PropertyNamesRefStringOnly::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        PropertyNamesRefStringOnly::is_valid(&instance),
        PropertyNamesRefStringOnly::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"s":{"type":"string","pattern":"^a"}},"type":"object","propertyNames":{"$ref":"#/$defs/s"}}"##,
    draft = Draft202012
)]
struct PropertyNamesRefPattern;

#[test_case(serde_json::json!({"abc":1}) ; "matching_key")]
#[test_case(serde_json::json!({"xbc":1}) ; "failing_key")]
#[test_case(serde_json::json!({}) ; "empty_object")]
fn test_property_names_ref_pattern(instance: serde_json::Value) {
    let schema = serde_json::json!({"$defs":{"s":{"type":"string","pattern":"^a"}},"type":"object","propertyNames":{"$ref":"#/$defs/s"}});
    assert_is_valid_parity(
        &schema,
        PropertyNamesRefPattern::is_valid(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &schema,
        PropertyNamesRefPattern::is_valid(&instance),
        PropertyNamesRefPattern::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(schema = r#"{"type":"object","propertyNames":{"type":"string"}}"#)]
struct PropertyNamesTypeStringOnly;

#[test_case(serde_json::json!({"k":1}) ; "keys_always_pass")]
#[test_case(serde_json::json!(5) ; "non_object")]
fn test_property_names_type_string_only(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","propertyNames":{"type":"string"}});
    assert_is_valid_parity(
        &schema,
        PropertyNamesTypeStringOnly::is_valid(&instance),
        &instance,
    );
}
