#![allow(clippy::needless_pass_by_value)]
use test_case::test_case;

fn is_currency_format(value: &str) -> bool {
    value
        .strip_prefix('$')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
}

fn only_literal_x(value: &str) -> bool {
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

#[jsonschema::validator(
    schema = r#"{"type":"integer","even":true}"#,
    keywords = { "even" => crate::even_factory }
)]
struct CustomKeywordValidator;

#[test_case(serde_json::json!(4) ; "even_integer")]
#[test_case(serde_json::json!(3) ; "odd_integer")]
#[test_case(serde_json::json!("x") ; "non_integer")]
fn test_custom_keyword_matches_dynamic(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"integer","even":true});
    let dynamic = jsonschema::options()
        .with_keyword("even", even_factory)
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        CustomKeywordValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
    );
    match (
        CustomKeywordValidator::validate(&instance),
        dynamic.validate(&instance),
    ) {
        (Ok(()), Ok(())) => {}
        (Err(generated), Err(expected)) => {
            assert_eq!(generated.to_string(), expected.to_string());
            assert_eq!(generated.schema_path(), expected.schema_path());
            assert_eq!(generated.instance_path(), expected.instance_path());
        }
        (generated, expected) => {
            panic!("validate() parity mismatch: generated={generated:?}, dynamic={expected:?}")
        }
    }
}

fn is_hex_content(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn check_prefixed(value: &str) -> bool {
    value.starts_with("p:")
}

// Signature fixed by `ValidationOptions::with_content_encoding`'s converter type.
#[allow(clippy::unnecessary_wraps)]
fn convert_prefixed(value: &str) -> Result<Option<String>, jsonschema::ValidationError<'static>> {
    Ok(value.strip_prefix("p:").map(str::to_string))
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

fn assert_validate_parity(
    generated_is_valid: bool,
    generated: Result<(), jsonschema::ValidationError<'_>>,
    dynamic: &jsonschema::Validator,
    instance: &serde_json::Value,
) {
    assert_eq!(generated_is_valid, dynamic.is_valid(instance));
    match (generated, dynamic.validate(instance)) {
        (Ok(()), Ok(())) => {}
        (Err(generated), Err(expected)) => {
            assert_eq!(generated.to_string(), expected.to_string());
            assert_eq!(generated.schema_path(), expected.schema_path());
            assert_eq!(generated.instance_path(), expected.instance_path());
        }
        (generated, expected) => {
            panic!("validate() parity mismatch: generated={generated:?}, dynamic={expected:?}")
        }
    }
}

#[test_case(serde_json::json!("deadBEEF") ; "valid_hex")]
#[test_case(serde_json::json!("xyz") ; "invalid_hex")]
#[test_case(serde_json::json!(5) ; "non_string")]
fn test_custom_content_media_type_matches_dynamic(instance: serde_json::Value) {
    let dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .with_content_media_type("text/hex", is_hex_content)
        .build(&serde_json::json!({"contentMediaType":"text/hex"}))
        .expect("valid schema");
    assert_validate_parity(
        CustomContentMediaTypeValidator::is_valid(&instance),
        CustomContentMediaTypeValidator::validate(&instance),
        &dynamic,
        &instance,
    );
}

#[test_case(serde_json::json!("p:payload") ; "valid_prefixed")]
#[test_case(serde_json::json!("payload") ; "missing_prefix")]
fn test_custom_content_encoding_matches_dynamic(instance: serde_json::Value) {
    let dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .with_content_encoding("prefixed", check_prefixed, convert_prefixed)
        .build(&serde_json::json!({"contentEncoding":"prefixed"}))
        .expect("valid schema");
    assert_validate_parity(
        CustomContentEncodingValidator::is_valid(&instance),
        CustomContentEncodingValidator::validate(&instance),
        &dynamic,
        &instance,
    );
}

#[test_case(serde_json::json!("p:{\"a\":1}") ; "decodes_to_json")]
#[test_case(serde_json::json!("p:not json") ; "decodes_to_non_json")]
#[test_case(serde_json::json!("no prefix") ; "conversion_fails")]
fn test_custom_encoding_with_builtin_media_matches_dynamic(instance: serde_json::Value) {
    let dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .with_content_encoding("prefixed", check_prefixed, convert_prefixed)
        .build(&serde_json::json!({"contentEncoding":"prefixed","contentMediaType":"application/json"}))
        .expect("valid schema");
    assert_validate_parity(
        CustomEncodingBuiltinMediaValidator::is_valid(&instance),
        CustomEncodingBuiltinMediaValidator::validate(&instance),
        &dynamic,
        &instance,
    );
}

#[test_case(serde_json::json!("aGVsbG8=") ; "valid_base64")]
#[test_case(serde_json::json!("!!!") ; "invalid_base64")]
fn test_builtin_content_encoding_matches_dynamic(instance: serde_json::Value) {
    let dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&serde_json::json!({"contentEncoding":"base64"}))
        .expect("valid schema");
    assert_validate_parity(
        BuiltinContentEncodingValidator::is_valid(&instance),
        BuiltinContentEncodingValidator::validate(&instance),
        &dynamic,
        &instance,
    );
}

// Content names are case-sensitive: `BASE64` is an unknown encoding and
// validates nothing, exactly like the runtime validator.
#[test]
fn test_unknown_uppercase_content_encoding_is_ignored() {
    let instance = serde_json::json!("!!!");
    let dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&serde_json::json!({"contentEncoding":"BASE64"}))
        .expect("valid schema");
    assert!(dynamic.is_valid(&instance));
    assert!(UppercaseContentEncodingValidator::is_valid(&instance));
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
        engine = regex,
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

#[jsonschema::validator(
    schema = r##"{"$defs":{"node":{"type":"array","items":{"$ref":"#/$defs/node"}}},"$ref":"#/$defs/node"}"##
)]
struct RecursiveNodeValidator;

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
        allow_display_text = false,
    }
)]
struct EmailOptionsConfiguredValidator;

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
        "email" => crate::only_literal_x,
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

// A required property whose `const` kinds disagree across `oneOf` branches must not
// be used as a discriminator (the emitted match would compare mismatched scalar kinds).
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
fn test_numeric_minimum_mixed_representations_match_dynamic(
    instance: serde_json::Value,
    expected: bool,
) {
    let schema = serde_json::json!({"type":"number","minimum":9_007_199_254_740_993_u64});
    let dynamic = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(dynamic.is_valid(&instance), expected);
    assert_eq!(
        NumericMinimumMixedRepresentationValidator::is_valid(&instance),
        expected
    );
}

#[cfg(feature = "arbitrary-precision")]
fn dynamic_valid(schema: &serde_json::Value, instance: &serde_json::Value) -> bool {
    jsonschema::validator_for(schema)
        .expect("valid schema")
        .is_valid(instance)
}

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"integer"}"#)]
struct ArbitraryPrecisionIntegerTypeValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"type":"integer"}"#, draft = referencing::Draft::Draft4)]
struct ArbitraryPrecisionDraft4IntegerTypeValidator;

// Integer typing for numbers outside the i64/u64/f64 range must match the
// runtime validator in both draft semantics.
#[cfg(feature = "arbitrary-precision")]
#[test_case("1e400" ; "huge_scientific_integer")]
#[test_case("10000000000000000000000000000000000001" ; "huge_plain_integer")]
#[test_case("1.0" ; "decimal_point_integer_value")]
#[test_case("1e308" ; "in_f64_range_scientific")]
#[test_case("1.5" ; "fractional")]
fn test_arbitrary_precision_integer_type_matches_dynamic(instance_json: &str) {
    let instance: serde_json::Value =
        serde_json::from_str(instance_json).expect("valid instance json");
    assert_eq!(
        ArbitraryPrecisionIntegerTypeValidator::is_valid(&instance),
        dynamic_valid(&serde_json::json!({"type":"integer"}), &instance),
        "draft 2020-12 divergence for {instance_json}"
    );
    let draft4_dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft4)
        .build(&serde_json::json!({"type":"integer"}))
        .expect("valid schema")
        .is_valid(&instance);
    assert_eq!(
        ArbitraryPrecisionDraft4IntegerTypeValidator::is_valid(&instance),
        draft4_dynamic,
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
fn test_arbitrary_precision_codegen_matches_dynamic(schema_json: &str, instance_json: &str) {
    let schema: serde_json::Value = serde_json::from_str(schema_json).expect("valid schema json");
    let instance: serde_json::Value =
        serde_json::from_str(instance_json).expect("valid instance json");
    let dynamic = dynamic_valid(&schema, &instance);

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
        _ => unreachable!("unknown schema in test matrix"),
    };

    assert_eq!(
        generated, dynamic,
        "codegen/dynamic mismatch for schema={schema_json}, instance={instance_json}"
    );
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
    let dynamic = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(
        OneOfDiscriminatorWithScalarBranchValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
    );
}

// JSON Schema `const: 1` matches `1.0`; the int-discriminator fast path must not
// reject float-valued integers that the full branch check would accept.
#[test_case(serde_json::json!({"tag": 1, "a": "x"}) ; "integer_form")]
#[test_case(serde_json::json!({"tag": 1.0, "a": "x"}) ; "float_integer_form")]
#[test_case(serde_json::json!({"tag": 2.0, "a": 3}) ; "float_integer_second_branch")]
#[test_case(serde_json::json!({"tag": 1.5}) ; "non_integer_tag")]
#[test_case(serde_json::json!({"tag": 3}) ; "unmatched_tag")]
#[test_case(serde_json::json!({"tag": 1.0, "a": 3}) ; "float_tag_body_mismatch")]
fn test_one_of_int_discriminator_matches_dynamic(instance: serde_json::Value) {
    let schema = serde_json::json!({
        "oneOf": [
            {"type":"object","required":["tag"],"properties":{"tag":{"const":1},"a":{"type":"string"}}},
            {"type":"object","required":["tag"],"properties":{"tag":{"const":2},"a":{"type":"integer"}}}
        ]
    });
    let dynamic = jsonschema::validator_for(&schema).expect("valid schema");
    let expected = dynamic.is_valid(&instance);
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
    let dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft4)
        .with_base_uri("json-schema:///root/main.json")
        .with_registry(&registry)
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        Draft4ModernIdIgnoredValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
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
fn test_email_options_codegen_matches_dynamic(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"string","format":"email"});
    let dynamic = jsonschema::options()
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
        dynamic.is_valid(&instance)
    );
}

#[test_case(serde_json::json!(1); "valid_integer")]
#[test_case(serde_json::json!(0); "minimum_violation")]
#[test_case(serde_json::json!("x"); "wrong_type")]
fn test_base_uri_codegen_matches_dynamic(instance: serde_json::Value) {
    let schema = serde_json::json!({"$ref":"defs.json#/$defs/item"});
    let defs_schema = serde_json::json!({"$defs":{"item":{"type":"integer","minimum":1}}});
    let defs_registry = jsonschema::Registry::new()
        .add("json-schema:///root/defs.json", &defs_schema)
        .expect("resource accepted")
        .prepare()
        .expect("registry build failed");
    let dynamic = jsonschema::options()
        .with_base_uri("json-schema:///root/main.json")
        .with_registry(&defs_registry)
        .build(&schema)
        .expect("valid schema");
    assert_eq!(
        BaseUriRelativeRefValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
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
fn test_dependencies_affect_unevaluated_properties_like_dynamic(
    instance: serde_json::Value,
    expected: bool,
) {
    let schema = serde_json::json!({
        "type":"object",
        "properties":{"foo":true},
        "dependencies":{"foo":{"properties":{"bar":true}}},
        "unevaluatedProperties":false
    });
    let dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft201909)
        .build(&schema)
        .expect("schema should build");

    assert_eq!(dynamic.is_valid(&instance), expected);
    assert_eq!(
        DependenciesUnevaluatedPropertiesValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        DependenciesUnevaluatedPropertiesValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
    );
}

#[test_case(jsonschema::Draft::Draft4, serde_json::json!(1), true ; "draft4_const_value")]
#[test_case(
    jsonschema::Draft::Draft4,
    serde_json::json!("anything"),
    true
; "draft4_const_is_ignored"
)]
fn test_dynamic_validator_draft_based_const_behavior(
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

#[test_case(jsonschema::Draft::Draft201909 ; "dynamic_2019_09_ref_siblings")]
#[test_case(jsonschema::Draft::Draft202012 ; "dynamic_2020_12_ref_siblings")]
fn test_dynamic_validator_ref_sibling_behavior(draft: jsonschema::Draft) {
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
; "dynamic_draft4_if_then_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft6,
    serde_json::json!({"if":{"const":1},"then":false}),
    serde_json::json!(1),
    true
; "dynamic_draft6_if_then_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft7,
    serde_json::json!({"type":"string","dependentSchemas":{"foo":{"required":["bar"]}}}),
    serde_json::json!({"foo":1}),
    false
; "dynamic_draft7_dependent_schemas_ignored_type_applies"
)]
#[test_case(
    jsonschema::Draft::Draft4,
    serde_json::json!({"propertyNames":{"pattern":"^x"}}),
    serde_json::json!({"y":1}),
    true
; "dynamic_draft4_property_names_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft4,
    serde_json::json!({"type":"string","contentEncoding":"base64"}),
    serde_json::json!("not base64"),
    true
; "dynamic_draft4_content_encoding_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft7,
    serde_json::json!({"type":"array","contains":{"type":"integer"},"minContains":2}),
    serde_json::json!([1]),
    true
; "dynamic_draft7_min_contains_ignored"
)]
#[test_case(
    jsonschema::Draft::Draft7,
    serde_json::json!({"type":"integer","minimum":0}),
    serde_json::json!(1.5),
    false
; "dynamic_integer_type_rejects_fractional_numbers_with_numeric_keywords"
)]
#[test_case(
    jsonschema::Draft::Draft7,
    serde_json::json!({"type":["integer","string"],"maximum":0}),
    serde_json::json!(1.5),
    false
; "dynamic_union_with_integer_and_numeric_keywords_rejects_fractional_numbers"
)]
fn test_dynamic_validator_draft_keyword_gating(
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

fn build_dynamic_with_resources(
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
    let dynamic = build_dynamic_with_resources(
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

    assert_eq!(dynamic.is_valid(&instance), expected);
    assert_eq!(
        NoValidationVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoValidationVocabularyValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
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
    let dynamic = build_dynamic_with_resources(
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

    assert_eq!(dynamic.is_valid(&instance), expected);
    assert_eq!(
        NoApplicatorVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoApplicatorVocabularyValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
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
    let dynamic = build_dynamic_with_resources(
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

    assert_eq!(dynamic.is_valid(&instance), expected);
    assert_eq!(
        NoUnevaluatedVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoUnevaluatedVocabularyValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
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
    let dynamic = build_dynamic_with_resources(
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

    assert_eq!(dynamic.is_valid(&instance), expected);
    assert_eq!(
        NoApplicatorArrayVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoApplicatorArrayVocabularyValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
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
    let dynamic = build_dynamic_with_resources(
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

    assert_eq!(dynamic.is_valid(&instance), expected);
    assert_eq!(
        NoUnevaluatedArrayVocabularyValidator::is_valid(&instance),
        expected
    );
    assert_eq!(
        NoUnevaluatedArrayVocabularyValidator::is_valid(&instance),
        dynamic.is_valid(&instance)
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

    // Compare against dynamic engine for the same schema.
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
    let dyn_v = jsonschema::options()
        .with_draft(referencing::Draft::Draft201909)
        .build(&schema)
        .expect("dynamic build ok");
    let dyn_err = dyn_v.validate(&invalid).expect_err("dynamic should fail");
    let dyn_sp = dyn_err.schema_path().to_string();

    assert_eq!(
        codegen_sp, dyn_sp,
        "codegen schema_path ({codegen_sp:?}) must match dynamic ({dyn_sp:?})"
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

// Mutual recursion via two $defs that reference each other — exercises a
// path where the cycle break must trip through the recursive/dynamic stack.
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
fn test_mutual_ref_validate_schema_path_matches_dynamic() {
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
    let dyn_v = jsonschema::options()
        .with_draft(referencing::Draft::Draft7)
        .build(&schema)
        .expect("dynamic build ok");
    let dyn_err = dyn_v.validate(&invalid).expect_err("dynamic should fail");
    let dyn_sp = dyn_err.schema_path().to_string();

    assert_eq!(codegen_sp, dyn_sp, "codegen vs dynamic schema_path");
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
    let dyn_v = jsonschema::options()
        .with_draft(referencing::Draft::Draft202012)
        .build(&schema)
        .expect("dynamic build ok");
    let dyn_err = dyn_v.validate(&invalid).expect_err("dynamic should fail");
    let dyn_sp = dyn_err.schema_path().to_string();

    assert_eq!(
        codegen_sp, dyn_sp,
        "codegen schema_path ({codegen_sp:?}) must match dynamic ({dyn_sp:?})"
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
fn test_prefix_items_error_order_matches_dynamic(instance: serde_json::Value) {
    let dynamic = jsonschema::options()
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
        &dynamic,
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
fn test_array_keyword_error_order_matches_dynamic(instance: serde_json::Value) {
    let dynamic = jsonschema::options()
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
        &dynamic,
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
fn test_object_count_bounds_error_matches_dynamic(instance: serde_json::Value) {
    let dynamic = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&serde_json::json!({"type": "object", "minProperties": 1, "maxProperties": 2}))
        .expect("valid schema");
    assert_validate_parity(
        ObjectCountBoundsValidator::is_valid(&instance),
        ObjectCountBoundsValidator::validate(&instance),
        &dynamic,
        &instance,
    );
}
