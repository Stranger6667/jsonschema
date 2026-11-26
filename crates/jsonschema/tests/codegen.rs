#![allow(clippy::needless_pass_by_value)]
use jsonschema::Draft;
use test_case::test_case;

fn is_currency_format(value: &str) -> bool {
    value
        .strip_prefix('$')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
}

fn is_literal_x(value: &str) -> bool {
    value == "x"
}

fn is_even_number(number: &serde_json::Number) -> bool {
    number.as_i64().is_some_and(|value| value % 2 == 0)
}

struct EvenKeyword;

impl<'i> jsonschema::Keyword<'i> for EvenKeyword {
    fn validate(
        &self,
        instance: &'i serde_json::Value,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(jsonschema::ValidationError::custom("number must be even"))
        }
    }

    fn is_valid(&self, instance: &'i serde_json::Value) -> bool {
        instance.as_u64().is_none_or(|n| n % 2 == 0)
    }
}

fn even_factory<'a>(
    _parent: &'a serde_json::Map<String, serde_json::Value>,
    value: &'a serde_json::Value,
    _path: jsonschema::paths::Location,
) -> Result<Box<dyn for<'i> jsonschema::Keyword<'i>>, jsonschema::ValidationError<'a>> {
    if value.as_bool() == Some(true) {
        Ok(Box::new(EvenKeyword))
    } else {
        Err(jsonschema::ValidationError::custom("`even` must be true"))
    }
}

struct MultiErrorKeyword;

impl<'i> jsonschema::Keyword<'i> for MultiErrorKeyword {
    fn validate(
        &self,
        instance: &'i serde_json::Value,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(jsonschema::ValidationError::custom("first"))
        }
    }

    fn is_valid(&self, instance: &'i serde_json::Value) -> bool {
        instance.as_i64().is_none_or(|n| n > 100)
    }

    fn iter_errors(
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
) -> Result<Box<dyn for<'i> jsonschema::Keyword<'i>>, jsonschema::ValidationError<'a>> {
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
            assert_eq!(generated.evaluation_path(), expected.evaluation_path());
            assert_eq!(
                generated.absolute_keyword_location(),
                expected.absolute_keyword_location()
            );
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
        assert_error_parity(generated, expected);
    }
}

fn error_context(
    kind: &jsonschema::error::ValidationErrorKind,
) -> Option<&[Vec<jsonschema::ValidationError<'static>>]> {
    match kind {
        jsonschema::error::ValidationErrorKind::AnyOf { context }
        | jsonschema::error::ValidationErrorKind::OneOfNotValid { context }
        | jsonschema::error::ValidationErrorKind::OneOfMultipleValid { context } => Some(context),
        _ => None,
    }
}

fn property_names_error(
    kind: &jsonschema::error::ValidationErrorKind,
) -> Option<&jsonschema::ValidationError<'static>> {
    match kind {
        jsonschema::error::ValidationErrorKind::PropertyNames { error } => Some(error),
        _ => None,
    }
}

fn assert_error_parity(
    generated: &jsonschema::ValidationError<'_>,
    expected: &jsonschema::ValidationError<'_>,
) {
    assert_eq!(generated.to_string(), expected.to_string());
    assert_eq!(generated.schema_path(), expected.schema_path());
    assert_eq!(generated.evaluation_path(), expected.evaluation_path());
    assert_eq!(
        generated.absolute_keyword_location(),
        expected.absolute_keyword_location()
    );
    assert_eq!(generated.instance_path(), expected.instance_path());
    assert_eq!(
        std::mem::discriminant(generated.kind()),
        std::mem::discriminant(expected.kind())
    );
    if let (Some(generated), Some(expected)) = (
        property_names_error(generated.kind()),
        property_names_error(expected.kind()),
    ) {
        assert_error_parity(generated, expected);
    }

    let generated_context = error_context(generated.kind());
    let expected_context = error_context(expected.kind());
    assert_eq!(
        generated_context.map(<[_]>::len),
        expected_context.map(<[_]>::len)
    );
    if let (Some(generated_context), Some(expected_context)) = (generated_context, expected_context)
    {
        for (generated_branch, expected_branch) in generated_context.iter().zip(expected_context) {
            assert_eq!(generated_branch.len(), expected_branch.len());
            for (generated, expected) in generated_branch.iter().zip(expected_branch) {
                assert_error_parity(generated, expected);
            }
        }
    }
}

#[jsonschema::validator(
    schema = r#"{"anyOf":[{"type":"integer","minimum":10},{"oneOf":[{"type":"string","minLength":3},{"type":"array","minItems":2}]}]}"#
)]
struct NestedAnyOfContextValidator;

#[test]
fn test_nested_any_of_error_context_matches_runtime() {
    let schema = serde_json::json!({
        "anyOf": [
            {"type": "integer", "minimum": 10},
            {"oneOf": [
                {"type": "string", "minLength": 3},
                {"type": "array", "minItems": 2}
            ]}
        ]
    });
    let instance = serde_json::json!(false);
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");

    assert_error_parity(
        &NestedAnyOfContextValidator::validate(&instance).expect_err("invalid instance"),
        &runtime.validate(&instance).expect_err("invalid instance"),
    );
    assert_iter_errors_parity(
        NestedAnyOfContextValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{}, {"anyOf":[{"type":"integer"},{"type":"string"}]},{"type":"boolean"}]}"#
)]
struct OneOfMultipleValidContextValidator;

#[test]
fn test_one_of_multiple_valid_error_context_matches_runtime() {
    let schema = serde_json::json!({
        "oneOf": [
            {},
            {"anyOf": [{"type": "integer"}, {"type": "string"}]},
            {"type": "boolean"}
        ]
    });
    let instance = serde_json::json!("value");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");

    assert_error_parity(
        &OneOfMultipleValidContextValidator::validate(&instance).expect_err("invalid instance"),
        &runtime.validate(&instance).expect_err("invalid instance"),
    );
    assert_iter_errors_parity(
        OneOfMultipleValidContextValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"$id": "http://example.com/root", "anyOf": [{"maxLength": 0}, {"minLength": 5}]}"#
)]
struct AnyOfContextInResource;
#[jsonschema::validator(
    schema = r#"{"$id": "http://example.com/root", "oneOf": [{"maxLength": 0}, {"minLength": 5}]}"#
)]
struct OneOfContextInResource;
#[jsonschema::validator(
    schema = r#"{"$id": "http://example.com/root", "oneOf": [{"maxLength": 5}, {"minLength": 1}, {"maxLength": 0}]}"#
)]
struct OneOfMultipleValidContextInResource;
#[jsonschema::validator(
    schema = r#"{"$id": "http://example.com/root", "items": {"$id": "http://example.com/item", "anyOf": [{"maxLength": 0}, {"minLength": 5}]}}"#
)]
struct AnyOfContextInEmbeddedResource;

// Errors of `anyOf` and `oneOf` branches are located in the resource of their keyword.
#[test_case(AnyOfContextInResource::validate, AnyOfContextInResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "anyOf": [{"maxLength": 0}, {"minLength": 5}]}), serde_json::json!("ab") ; "any_of")]
#[test_case(OneOfContextInResource::validate, OneOfContextInResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "oneOf": [{"maxLength": 0}, {"minLength": 5}]}), serde_json::json!("ab") ; "one_of")]
#[test_case(OneOfMultipleValidContextInResource::validate, OneOfMultipleValidContextInResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "oneOf": [{"maxLength": 5}, {"minLength": 1}, {"maxLength": 0}]}), serde_json::json!("ab") ; "one_of_multiple_valid")]
#[test_case(AnyOfContextInEmbeddedResource::validate, AnyOfContextInEmbeddedResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "items": {"$id": "http://example.com/item", "anyOf": [{"maxLength": 0}, {"minLength": 5}]}}), serde_json::json!(["ab"]) ; "any_of_in_embedded_resource")]
fn test_error_context_locations_match_runtime(
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    schema: serde_json::Value,
    instance: serde_json::Value,
) {
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_error_parity(
        &validate(&instance).expect_err("invalid instance"),
        &runtime.validate(&instance).expect_err("invalid instance"),
    );
    assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"circle":{"type":"object","required":["kind","radius"],"properties":{"kind":{"const":"circle"},"radius":{"type":"number"}}},"square":{"type":"object","required":["kind","side"],"properties":{"kind":{"const":"square"},"side":{"type":"number"}}}},"oneOf":[{"$ref":"#/$defs/circle"},{"$ref":"#/$defs/square"}]}"##
)]
struct OneOfDiscriminatorContextValidator;

#[test]
fn test_one_of_discriminator_error_context_matches_runtime() {
    let schema = serde_json::json!({
        "$defs": {
            "circle": {
                "type": "object",
                "required": ["kind", "radius"],
                "properties": {"kind": {"const": "circle"}, "radius": {"type": "number"}}
            },
            "square": {
                "type": "object",
                "required": ["kind", "side"],
                "properties": {"kind": {"const": "square"}, "side": {"type": "number"}}
            }
        },
        "oneOf": [{"$ref": "#/$defs/circle"}, {"$ref": "#/$defs/square"}]
    });
    let instance = serde_json::json!({"kind": "circle", "radius": "large"});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");

    assert_error_parity(
        &OneOfDiscriminatorContextValidator::validate(&instance).expect_err("invalid instance"),
        &runtime.validate(&instance).expect_err("invalid instance"),
    );
    assert_iter_errors_parity(
        OneOfDiscriminatorContextValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","required":["kind","radius"],"properties":{"kind":{"type":"string","const":"circle","minLength":8},"radius":{"type":"number"}}},{"type":"object","required":["kind"],"properties":{"kind":{"const":"square"}}}]}"#
)]
struct OneOfDirectDiscriminatorContextValidator;

#[test]
fn test_one_of_direct_discriminator_keeps_non_implied_checks_and_context() {
    let schema = serde_json::json!({
        "oneOf": [
            {
                "type": "object",
                "required": ["kind", "radius"],
                "properties": {
                    "kind": {"type": "string", "const": "circle", "minLength": 8},
                    "radius": {"type": "number"}
                }
            },
            {
                "type": "object",
                "required": ["kind"],
                "properties": {"kind": {"const": "square"}}
            }
        ]
    });
    let instance = serde_json::json!({"kind": "circle", "radius": 1});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");

    assert_error_parity(
        &OneOfDirectDiscriminatorContextValidator::validate(&instance)
            .expect_err("invalid instance"),
        &runtime.validate(&instance).expect_err("invalid instance"),
    );
    assert_iter_errors_parity(
        OneOfDirectDiscriminatorContextValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"node":{"$dynamicAnchor":"node","type":"integer"}},"anyOf":[{"$dynamicRef":"#node"},{"type":"string"}]}"##
)]
struct AnyOfDynamicRefContextValidator;

// A Draft 4 resource, whose `id` sets a plain-name anchor.
#[jsonschema::validator(
    schema = r##"{"$ref":"http://example.com/draft4.json#integer"}"##,
    draft = referencing::Draft::Draft202012,
    resources = { "http://example.com/draft4.json" => { schema = r##"{"$schema":"http://json-schema.org/draft-04/schema#","id":"http://example.com/draft4.json","definitions":{"integer":{"id":"#integer","type":"integer"}}}"## } }
)]
struct ResourceInItsOwnDraftValidator;

#[test_case(serde_json::json!(1) ; "integer")]
#[test_case(serde_json::json!("a") ; "string")]
fn test_resource_in_its_own_draft_matches_runtime(instance: serde_json::Value) {
    let resource = serde_json::json!({"$schema": "http://json-schema.org/draft-04/schema#", "id": "http://example.com/draft4.json", "definitions": {"integer": {"id": "#integer", "type": "integer"}}});
    let registry = jsonschema::Registry::new()
        .add("http://example.com/draft4.json", &resource)
        .expect("resource should be accepted")
        .prepare()
        .expect("registry builds");
    let runtime = jsonschema::options()
        .with_draft(referencing::Draft::Draft202012)
        .with_registry(&registry)
        .build(&serde_json::json!({"$ref": "http://example.com/draft4.json#integer"}))
        .expect("valid schema");
    assert_validate_parity(
        ResourceInItsOwnDraftValidator::is_valid(&instance),
        ResourceInItsOwnDraftValidator::validate(&instance),
        &runtime,
        &instance,
    );
}

// A `$ref` to the bundled meta-schema of another draft than the root's
#[jsonschema::validator(
    schema = r##"{"$ref":"https://json-schema.org/draft/2020-12/schema"}"##,
    draft = referencing::Draft::Draft7
)]
struct Draft7RefToDraft202012MetaSchemaValidator;

#[jsonschema::validator(schema = r##"{"$ref":"http://json-schema.org/draft-07/schema#"}"##)]
struct Draft202012RefToDraft7MetaSchemaValidator;

#[jsonschema::validator(
    schema = r##"{"$ref":"https://json-schema.org/draft/2019-09/meta/validation"}"##,
    draft = referencing::Draft::Draft4
)]
struct Draft4RefToDraft201909VocabularyMetaSchemaValidator;

#[test_case(Draft7RefToDraft202012MetaSchemaValidator::is_valid, Draft7RefToDraft202012MetaSchemaValidator::validate, referencing::Draft::Draft7, "https://json-schema.org/draft/2020-12/schema", serde_json::json!({"type": "string"}) ; "draft 7 to 2020-12 accepts")]
#[test_case(Draft7RefToDraft202012MetaSchemaValidator::is_valid, Draft7RefToDraft202012MetaSchemaValidator::validate, referencing::Draft::Draft7, "https://json-schema.org/draft/2020-12/schema", serde_json::json!({"type": 12}) ; "draft 7 to 2020-12 rejects")]
#[test_case(Draft202012RefToDraft7MetaSchemaValidator::is_valid, Draft202012RefToDraft7MetaSchemaValidator::validate, referencing::Draft::Draft202012, "http://json-schema.org/draft-07/schema#", serde_json::json!({"type": "string"}) ; "2020-12 to draft 7 accepts")]
#[test_case(Draft202012RefToDraft7MetaSchemaValidator::is_valid, Draft202012RefToDraft7MetaSchemaValidator::validate, referencing::Draft::Draft202012, "http://json-schema.org/draft-07/schema#", serde_json::json!({"type": 12}) ; "2020-12 to draft 7 rejects")]
#[test_case(Draft4RefToDraft201909VocabularyMetaSchemaValidator::is_valid, Draft4RefToDraft201909VocabularyMetaSchemaValidator::validate, referencing::Draft::Draft4, "https://json-schema.org/draft/2019-09/meta/validation", serde_json::json!({"type": "string"}) ; "draft 4 to 2019-09 vocabulary accepts")]
#[test_case(Draft4RefToDraft201909VocabularyMetaSchemaValidator::is_valid, Draft4RefToDraft201909VocabularyMetaSchemaValidator::validate, referencing::Draft::Draft4, "https://json-schema.org/draft/2019-09/meta/validation", serde_json::json!({"type": 12}) ; "draft 4 to 2019-09 vocabulary rejects")]
fn test_ref_to_meta_schema_of_another_draft_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    draft: referencing::Draft,
    target: &str,
    instance: serde_json::Value,
) {
    let runtime = jsonschema::options()
        .with_draft(draft)
        .offline()
        .build(&serde_json::json!({"$ref": target}))
        .expect("valid schema");
    assert_validate_parity(
        is_valid(&instance),
        validate(&instance),
        &runtime,
        &instance,
    );
}

// `myobject` is reached by `$ref` through the outer document, then by `$recursiveRef` within its own.
#[jsonschema::validator(
    schema = r##"{"$defs":{"myobject":{"$id":"myobject.json","$recursiveAnchor":true,"anyOf":[{"type":"string"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}},"anyOf":[{"type":"integer"},{"$ref":"#/$defs/myobject"}]}"##,
    draft = referencing::Draft::Draft201909
)]
struct RecursiveRefThroughDefinitionContextValidator;

#[test_case(serde_json::json!({"foo": 1}) ; "one level")]
#[test_case(serde_json::json!({"foo": {"bar": 1}}) ; "two levels")]
fn test_recursive_ref_through_definition_error_context_matches_runtime(
    instance: serde_json::Value,
) {
    let schema = serde_json::json!({"$defs": {"myobject": {"$id": "myobject.json", "$recursiveAnchor": true, "anyOf": [{"type": "string"}, {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}]}}, "anyOf": [{"type": "integer"}, {"$ref": "#/$defs/myobject"}]});
    let runtime = jsonschema::options()
        .with_draft(referencing::Draft::Draft201909)
        .build(&schema)
        .expect("valid schema");
    assert_error_parity(
        &RecursiveRefThroughDefinitionContextValidator::validate(&instance)
            .expect_err("invalid instance"),
        &runtime.validate(&instance).expect_err("invalid instance"),
    );
    assert_iter_errors_parity(
        RecursiveRefThroughDefinitionContextValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

#[test]
fn test_any_of_dynamic_ref_error_context_matches_runtime() {
    let schema = serde_json::json!({
        "$defs": {"node": {"$dynamicAnchor": "node", "type": "integer"}},
        "anyOf": [{"$dynamicRef": "#node"}, {"type": "string"}]
    });
    let instance = serde_json::json!(false);
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");

    assert_error_parity(
        &AnyOfDynamicRefContextValidator::validate(&instance).expect_err("invalid instance"),
        &runtime.validate(&instance).expect_err("invalid instance"),
    );
    assert_iter_errors_parity(
        AnyOfDynamicRefContextValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

// `wrap` enters the dynamic scope on the way back to the `$ref` target `cycle` and binds `node`.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"start","$defs":{"start":{"$id":"start","$ref":"#/$defs/cycle","$defs":{"cycle":{"properties":{"next":{"$ref":"wrap"}},"additionalProperties":{"$dynamicRef":"leaf#node"}}}},"leaf":{"$id":"leaf","$dynamicAnchor":"node","type":"integer"},"wrap":{"$id":"wrap","$dynamicAnchor":"node","type":"object","$ref":"start#/$defs/cycle"}}}"##
)]
struct DynamicAnchorBoundInsideRefCycle;

#[test_case(serde_json::json!({"a": 1}) ; "leaf binds before wrap")]
#[test_case(serde_json::json!({"a": {}}) ; "leaf rejects before wrap")]
#[test_case(serde_json::json!({"next": {"a": {}}}) ; "wrap binds")]
#[test_case(serde_json::json!({"next": {"a": 1}}) ; "wrap rejects")]
#[test_case(serde_json::json!({"next": {"next": {"a": 1}}}) ; "wrap rejects twice in")]
fn test_dynamic_anchor_bound_inside_ref_cycle_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "start", "$defs": {"start": {"$id": "start", "$ref": "#/$defs/cycle", "$defs": {"cycle": {"properties": {"next": {"$ref": "wrap"}}, "additionalProperties": {"$dynamicRef": "leaf#node"}}}}, "leaf": {"$id": "leaf", "$dynamicAnchor": "node", "type": "integer"}, "wrap": {"$id": "wrap", "$dynamicAnchor": "node", "type": "object", "$ref": "start#/$defs/cycle"}}});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        DynamicAnchorBoundInsideRefCycle::is_valid(&instance),
        DynamicAnchorBoundInsideRefCycle::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        DynamicAnchorBoundInsideRefCycle::iter_errors(&instance),
        &runtime,
        &instance,
    );
    assert_eq!(
        evaluation_outputs(&DynamicAnchorBoundInsideRefCycle::evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

const LARGE_SUBSCHEMAS: &str = r#"{"oneOf":[{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"},"c":{"type":"integer"},"d":{"type":"integer"},"e":{"type":"integer"},"f":{"type":"integer"},"g":{"type":"integer"},"h":{"type":"integer"},"i":{"type":"integer"},"j":{"type":"integer"},"k":{"type":"integer"},"l":{"type":"integer"},"m":{"type":"integer"},"n":{"type":"integer"},"o":{"type":"integer"},"p":{"type":"integer"}},"required":["a"]},{"type":"array","items":{"properties":{"a":{"minimum":0},"b":{"minimum":0},"c":{"minimum":0},"d":{"minimum":0},"e":{"minimum":0},"f":{"minimum":0},"g":{"minimum":0},"h":{"minimum":0},"i":{"minimum":0},"j":{"minimum":0},"k":{"minimum":0},"l":{"minimum":0},"m":{"minimum":0},"n":{"minimum":0},"o":{"minimum":0},"p":{"minimum":0}}}},{"type":"null"}]}"#;

// A `oneOf` branch and an `items` subschema with code large enough for functions of their own.
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"},"c":{"type":"integer"},"d":{"type":"integer"},"e":{"type":"integer"},"f":{"type":"integer"},"g":{"type":"integer"},"h":{"type":"integer"},"i":{"type":"integer"},"j":{"type":"integer"},"k":{"type":"integer"},"l":{"type":"integer"},"m":{"type":"integer"},"n":{"type":"integer"},"o":{"type":"integer"},"p":{"type":"integer"}},"required":["a"]},{"type":"array","items":{"properties":{"a":{"minimum":0},"b":{"minimum":0},"c":{"minimum":0},"d":{"minimum":0},"e":{"minimum":0},"f":{"minimum":0},"g":{"minimum":0},"h":{"minimum":0},"i":{"minimum":0},"j":{"minimum":0},"k":{"minimum":0},"l":{"minimum":0},"m":{"minimum":0},"n":{"minimum":0},"o":{"minimum":0},"p":{"minimum":0}}}},{"type":"null"}]}"#
)]
struct LargeSubschemasValidator;

#[test_case(serde_json::json!({"a": 1, "p": 2}) ; "object branch accepts")]
#[test_case(serde_json::json!({"a": 1, "p": "x"}) ; "object branch rejects a member")]
#[test_case(serde_json::json!({"b": 1}) ; "object branch misses a")]
#[test_case(serde_json::json!([{"a": 1}, {"p": 0}]) ; "array branch accepts")]
#[test_case(serde_json::json!([{"a": 1}, {"p": -1}]) ; "array branch rejects an item")]
#[test_case(serde_json::json!(null) ; "null branch")]
#[test_case(serde_json::json!("x") ; "no branch")]
fn test_large_subschemas_match_runtime(instance: serde_json::Value) {
    let schema: serde_json::Value = serde_json::from_str(LARGE_SUBSCHEMAS).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(
        evaluation_outputs(&LargeSubschemasValidator::evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

const NO_VALIDATION_META_SCHEMA: &str = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"#;
const SUBSCHEMA_IN_DRAFT_4: &str = r#"{"properties":{"a":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"}}}"#;
const BRANCH_IN_DRAFT_4: &str = r#"{"anyOf":[{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},{"type":"string"}]}"#;
const SUBSCHEMA_IN_DRAFT_7_BESIDE_REF: &str = r##"{"properties":{"a":{"$schema":"http://json-schema.org/draft-07/schema#","$ref":"#/$defs/x","minimum":5}},"$defs":{"x":{}}}"##;
const EMBEDDED_RESOURCE_IN_DRAFT_7: &str = r#"{"properties":{"a":{"$id":"http://example.com/a","$schema":"http://json-schema.org/draft-07/schema#","dependentRequired":{"x":["y"]},"minimum":2}}}"#;
const EMBEDDED_RESOURCE_IN_DRAFT_4: &str = r#"{"properties":{"a":{"$id":"http://example.com/a","$schema":"http://json-schema.org/draft-04/schema#","type":"integer"}}}"#;
const SUBSCHEMA_IN_DRAFT_4_BELOW_REF_TARGET: &str = r##"{"$ref":"#a","$defs":{"a":{"$anchor":"a","properties":{"b":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"}}}}}"##;
const SUBSCHEMA_WITH_META_SCHEMA: &str =
    r#"{"properties":{"a":{"$schema":"json-schema:///meta/no-validation","minimum":5}}}"#;
const EMBEDDED_RESOURCE_WITH_META_SCHEMA: &str = r#"{"properties":{"a":{"$id":"http://example.com/a","$schema":"json-schema:///meta/no-validation","minimum":5}}}"#;
const SUBSCHEMA_WITH_META_SCHEMA_THROUGH_REF: &str = r##"{"properties":{"inline":{"$schema":"json-schema:///meta/no-validation","allOf":[{"minimum":5}]},"direct":{"$ref":"#/$defs/x"},"child":{"$ref":"#/$defs/x/allOf/0"},"anchor":{"$ref":"#c"},"dynamic":{"$dynamicRef":"#d"}},"$defs":{"x":{"$schema":"json-schema:///meta/no-validation","allOf":[{"$anchor":"c","minimum":5},{"$dynamicAnchor":"d","minimum":5}]}}}"##;
const SUBSCHEMA_UNDER_META_SCHEMA_THROUGH_REF: &str = r##"{"$schema":"json-schema:///meta/no-validation","properties":{"inline":{"$schema":"https://json-schema.org/draft/2019-09/schema","allOf":[{"minimum":5}]},"child":{"$ref":"#/$defs/x/allOf/0"},"anchor":{"$ref":"#c"},"resource":{"$ref":"https://example.com/r"}},"$defs":{"x":{"$schema":"https://json-schema.org/draft/2019-09/schema","allOf":[{"$anchor":"c","minimum":5}]},"r":{"$id":"https://example.com/r","allOf":[{"minimum":5}]}}}"##;
const EVALUATED_BRANCH_UNDER_META_SCHEMA: &str = r#"{"$schema":"json-schema:///meta/no-validation","anyOf":[{"properties":{"a":{}},"allOf":[{"properties":{"a":{"minimum":5}}}]},{"properties":{"b":{}}}],"unevaluatedProperties":false}"#;
const SUBSCHEMA_IN_DRAFT_4_IN_RESOURCE: &str = r#"{"$ref":"json-schema:///other"}"#;
const RESOURCE_WITH_SUBSCHEMA_IN_DRAFT_4: &str = r#"{"properties":{"a":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"}}}"#;
const SUBSCHEMA_IN_ROOT_DRAFT_UNDER_ANOTHER: &str = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","dependentRequired":{"x":["y"]},"properties":{"a":{"$schema":"https://json-schema.org/draft/2020-12/schema","dependentRequired":{"x":["y"]}}}}"#;

#[jsonschema::validator(
    schema = r#"{"properties":{"a":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"}}}"#
)]
struct SubschemaInDraft4;

#[jsonschema::validator(
    schema = r#"{"anyOf":[{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},{"type":"string"}]}"#
)]
struct BranchInDraft4;

// Draft 7 ignores the keywords beside `$ref`.
#[jsonschema::validator(
    schema = r##"{"properties":{"a":{"$schema":"http://json-schema.org/draft-07/schema#","$ref":"#/$defs/x","minimum":5}},"$defs":{"x":{}}}"##
)]
struct SubschemaInDraft7BesideRef;

// Draft 7 has no `dependentRequired`.
#[jsonschema::validator(
    schema = r#"{"properties":{"a":{"$id":"http://example.com/a","$schema":"http://json-schema.org/draft-07/schema#","dependentRequired":{"x":["y"]},"minimum":2}}}"#
)]
struct EmbeddedResourceInDraft7;

// Draft 4 names a resource with `id`, so `$id` is an unknown keyword there.
#[jsonschema::validator(
    schema = r#"{"properties":{"a":{"$id":"http://example.com/a","$schema":"http://json-schema.org/draft-04/schema#","type":"integer"}}}"#
)]
struct EmbeddedResourceInDraft4;

#[jsonschema::validator(
    schema = r##"{"$ref":"#a","$defs":{"a":{"$anchor":"a","properties":{"b":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"}}}}}"##
)]
struct SubschemaInDraft4BelowRefTarget;

#[jsonschema::validator(
    schema = r#"{"properties":{"a":{"$schema":"json-schema:///meta/no-validation","minimum":5}}}"#,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct SubschemaWithMetaSchema;

#[jsonschema::validator(
    schema = r#"{"properties":{"a":{"$id":"http://example.com/a","$schema":"json-schema:///meta/no-validation","minimum":5}}}"#,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct EmbeddedResourceWithMetaSchema;

#[jsonschema::validator(
    schema = r##"{"properties":{"inline":{"$schema":"json-schema:///meta/no-validation","allOf":[{"minimum":5}]},"direct":{"$ref":"#/$defs/x"},"child":{"$ref":"#/$defs/x/allOf/0"},"anchor":{"$ref":"#c"},"dynamic":{"$dynamicRef":"#d"}},"$defs":{"x":{"$schema":"json-schema:///meta/no-validation","allOf":[{"$anchor":"c","minimum":5},{"$dynamicAnchor":"d","minimum":5}]}}}"##,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct SubschemaWithMetaSchemaThroughRef;

#[jsonschema::validator(
    schema = r##"{"$schema":"json-schema:///meta/no-validation","properties":{"inline":{"$schema":"https://json-schema.org/draft/2019-09/schema","allOf":[{"minimum":5}]},"child":{"$ref":"#/$defs/x/allOf/0"},"anchor":{"$ref":"#c"},"resource":{"$ref":"https://example.com/r"}},"$defs":{"x":{"$schema":"https://json-schema.org/draft/2019-09/schema","allOf":[{"$anchor":"c","minimum":5}]},"r":{"$id":"https://example.com/r","allOf":[{"minimum":5}]}}}"##,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct SubschemaUnderMetaSchemaThroughRef;

// The first branch evaluates `a` only where it accepts, with `minimum` off.
#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-validation","anyOf":[{"properties":{"a":{}},"allOf":[{"properties":{"a":{"minimum":5}}}]},{"properties":{"b":{}}}],"unevaluatedProperties":false}"#,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":false,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true,"https://json-schema.org/draft/2020-12/vocab/format-annotation":true}}"# },
    }
)]
struct EvaluatedBranchUnderMetaSchema;

#[jsonschema::validator(
    schema = r#"{"$ref":"json-schema:///other"}"#,
    resources = {
        "json-schema:///other" => { schema = r#"{"properties":{"a":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"}}}"# },
    }
)]
struct SubschemaInDraft4InResource;

// The root stays in the configured draft; the subschema is in the draft its `$schema` names.
#[jsonschema::validator(
    schema = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","dependentRequired":{"x":["y"]},"properties":{"a":{"$schema":"https://json-schema.org/draft/2020-12/schema","dependentRequired":{"x":["y"]}}}}"#,
    draft = referencing::Draft::Draft7
)]
struct SubschemaInRootDraftUnderAnother;

#[test_case(SubschemaInDraft4::is_valid, SubschemaInDraft4::validate, SubschemaInDraft4::iter_errors, SubschemaInDraft4::evaluate, SUBSCHEMA_IN_DRAFT_4, referencing::Draft::Draft202012, serde_json::json!({"a": 1.0}) ; "draft4_subschema_float")]
#[test_case(SubschemaInDraft4::is_valid, SubschemaInDraft4::validate, SubschemaInDraft4::iter_errors, SubschemaInDraft4::evaluate, SUBSCHEMA_IN_DRAFT_4, referencing::Draft::Draft202012, serde_json::json!({"a": 1}) ; "draft4_subschema_integer")]
#[test_case(BranchInDraft4::is_valid, BranchInDraft4::validate, BranchInDraft4::iter_errors, BranchInDraft4::evaluate, BRANCH_IN_DRAFT_4, referencing::Draft::Draft202012, serde_json::json!(1.0) ; "draft4_branch_float")]
#[test_case(BranchInDraft4::is_valid, BranchInDraft4::validate, BranchInDraft4::iter_errors, BranchInDraft4::evaluate, BRANCH_IN_DRAFT_4, referencing::Draft::Draft202012, serde_json::json!(1) ; "draft4_branch_integer")]
#[test_case(SubschemaInDraft7BesideRef::is_valid, SubschemaInDraft7BesideRef::validate, SubschemaInDraft7BesideRef::iter_errors, SubschemaInDraft7BesideRef::evaluate, SUBSCHEMA_IN_DRAFT_7_BESIDE_REF, referencing::Draft::Draft202012, serde_json::json!({"a": 1}) ; "draft7_keyword_beside_ref")]
#[test_case(EmbeddedResourceInDraft7::is_valid, EmbeddedResourceInDraft7::validate, EmbeddedResourceInDraft7::iter_errors, EmbeddedResourceInDraft7::evaluate, EMBEDDED_RESOURCE_IN_DRAFT_7, referencing::Draft::Draft202012, serde_json::json!({"a": {"x": 1}}) ; "draft7_resource_unknown_keyword")]
#[test_case(EmbeddedResourceInDraft7::is_valid, EmbeddedResourceInDraft7::validate, EmbeddedResourceInDraft7::iter_errors, EmbeddedResourceInDraft7::evaluate, EMBEDDED_RESOURCE_IN_DRAFT_7, referencing::Draft::Draft202012, serde_json::json!({"a": 1}) ; "draft7_resource_error")]
#[test_case(EmbeddedResourceInDraft4::is_valid, EmbeddedResourceInDraft4::validate, EmbeddedResourceInDraft4::iter_errors, EmbeddedResourceInDraft4::evaluate, EMBEDDED_RESOURCE_IN_DRAFT_4, referencing::Draft::Draft202012, serde_json::json!({"a": 1.0}) ; "draft4_resource_float")]
#[test_case(SubschemaInDraft4BelowRefTarget::is_valid, SubschemaInDraft4BelowRefTarget::validate, SubschemaInDraft4BelowRefTarget::iter_errors, SubschemaInDraft4BelowRefTarget::evaluate, SUBSCHEMA_IN_DRAFT_4_BELOW_REF_TARGET, referencing::Draft::Draft202012, serde_json::json!({"b": 1.0}) ; "draft4_subschema_below_ref_target")]
#[test_case(SubschemaWithMetaSchema::is_valid, SubschemaWithMetaSchema::validate, SubschemaWithMetaSchema::iter_errors, SubschemaWithMetaSchema::evaluate, SUBSCHEMA_WITH_META_SCHEMA, referencing::Draft::Draft202012, serde_json::json!({"a": 1}) ; "meta_schema_subschema")]
#[test_case(EmbeddedResourceWithMetaSchema::is_valid, EmbeddedResourceWithMetaSchema::validate, EmbeddedResourceWithMetaSchema::iter_errors, EmbeddedResourceWithMetaSchema::evaluate, EMBEDDED_RESOURCE_WITH_META_SCHEMA, referencing::Draft::Draft202012, serde_json::json!({"a": 1}) ; "meta_schema_resource")]
#[test_case(SubschemaWithMetaSchemaThroughRef::is_valid, SubschemaWithMetaSchemaThroughRef::validate, SubschemaWithMetaSchemaThroughRef::iter_errors, SubschemaWithMetaSchemaThroughRef::evaluate, SUBSCHEMA_WITH_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"inline": 1}) ; "meta_schema_subschema_inline_beside_refs")]
#[test_case(SubschemaWithMetaSchemaThroughRef::is_valid, SubschemaWithMetaSchemaThroughRef::validate, SubschemaWithMetaSchemaThroughRef::iter_errors, SubschemaWithMetaSchemaThroughRef::evaluate, SUBSCHEMA_WITH_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"direct": 1}) ; "meta_schema_subschema_through_ref")]
#[test_case(SubschemaWithMetaSchemaThroughRef::is_valid, SubschemaWithMetaSchemaThroughRef::validate, SubschemaWithMetaSchemaThroughRef::iter_errors, SubschemaWithMetaSchemaThroughRef::evaluate, SUBSCHEMA_WITH_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"child": 1}) ; "meta_schema_subschema_child_through_ref")]
#[test_case(SubschemaWithMetaSchemaThroughRef::is_valid, SubschemaWithMetaSchemaThroughRef::validate, SubschemaWithMetaSchemaThroughRef::iter_errors, SubschemaWithMetaSchemaThroughRef::evaluate, SUBSCHEMA_WITH_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"anchor": 1}) ; "meta_schema_subschema_child_through_anchor")]
#[test_case(SubschemaWithMetaSchemaThroughRef::is_valid, SubschemaWithMetaSchemaThroughRef::validate, SubschemaWithMetaSchemaThroughRef::iter_errors, SubschemaWithMetaSchemaThroughRef::evaluate, SUBSCHEMA_WITH_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"dynamic": 1}) ; "meta_schema_subschema_child_through_dynamic_anchor")]
#[test_case(SubschemaUnderMetaSchemaThroughRef::is_valid, SubschemaUnderMetaSchemaThroughRef::validate, SubschemaUnderMetaSchemaThroughRef::iter_errors, SubschemaUnderMetaSchemaThroughRef::evaluate, SUBSCHEMA_UNDER_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"inline": 1}) ; "standard_subschema_under_meta_schema_inline")]
#[test_case(SubschemaUnderMetaSchemaThroughRef::is_valid, SubschemaUnderMetaSchemaThroughRef::validate, SubschemaUnderMetaSchemaThroughRef::iter_errors, SubschemaUnderMetaSchemaThroughRef::evaluate, SUBSCHEMA_UNDER_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"child": 1}) ; "standard_subschema_under_meta_schema_child_through_ref")]
#[test_case(SubschemaUnderMetaSchemaThroughRef::is_valid, SubschemaUnderMetaSchemaThroughRef::validate, SubschemaUnderMetaSchemaThroughRef::iter_errors, SubschemaUnderMetaSchemaThroughRef::evaluate, SUBSCHEMA_UNDER_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"anchor": 1}) ; "standard_subschema_under_meta_schema_child_through_anchor")]
#[test_case(SubschemaUnderMetaSchemaThroughRef::is_valid, SubschemaUnderMetaSchemaThroughRef::validate, SubschemaUnderMetaSchemaThroughRef::iter_errors, SubschemaUnderMetaSchemaThroughRef::evaluate, SUBSCHEMA_UNDER_META_SCHEMA_THROUGH_REF, referencing::Draft::Draft202012, serde_json::json!({"resource": 1}) ; "resource_under_meta_schema_through_ref")]
#[test_case(EvaluatedBranchUnderMetaSchema::is_valid, EvaluatedBranchUnderMetaSchema::validate, EvaluatedBranchUnderMetaSchema::iter_errors, EvaluatedBranchUnderMetaSchema::evaluate, EVALUATED_BRANCH_UNDER_META_SCHEMA, referencing::Draft::Draft202012, serde_json::json!({"a": 1, "b": 1}) ; "evaluated_branch_under_meta_schema")]
#[test_case(EvaluatedBranchUnderMetaSchema::is_valid, EvaluatedBranchUnderMetaSchema::validate, EvaluatedBranchUnderMetaSchema::iter_errors, EvaluatedBranchUnderMetaSchema::evaluate, EVALUATED_BRANCH_UNDER_META_SCHEMA, referencing::Draft::Draft202012, serde_json::json!({"c": 1}) ; "unevaluated_member_under_meta_schema")]
#[test_case(SubschemaInDraft4InResource::is_valid, SubschemaInDraft4InResource::validate, SubschemaInDraft4InResource::iter_errors, SubschemaInDraft4InResource::evaluate, SUBSCHEMA_IN_DRAFT_4_IN_RESOURCE, referencing::Draft::Draft202012, serde_json::json!({"a": 1.0}) ; "draft4_subschema_in_resource")]
#[test_case(SubschemaInRootDraftUnderAnother::is_valid, SubschemaInRootDraftUnderAnother::validate, SubschemaInRootDraftUnderAnother::iter_errors, SubschemaInRootDraftUnderAnother::evaluate, SUBSCHEMA_IN_ROOT_DRAFT_UNDER_ANOTHER, referencing::Draft::Draft7, serde_json::json!({"x": 1}) ; "configured_draft_root")]
#[test_case(SubschemaInRootDraftUnderAnother::is_valid, SubschemaInRootDraftUnderAnother::validate, SubschemaInRootDraftUnderAnother::iter_errors, SubschemaInRootDraftUnderAnother::evaluate, SUBSCHEMA_IN_ROOT_DRAFT_UNDER_ANOTHER, referencing::Draft::Draft7, serde_json::json!({"a": {"x": 1}}) ; "configured_draft_subschema")]
fn test_subschema_in_the_dialect_its_schema_names_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
    draft: referencing::Draft,
    instance: serde_json::Value,
) {
    let meta_schema: serde_json::Value =
        serde_json::from_str(NO_VALIDATION_META_SCHEMA).expect("valid JSON");
    let other: serde_json::Value =
        serde_json::from_str(RESOURCE_WITH_SUBSCHEMA_IN_DRAFT_4).expect("valid JSON");
    let registry = jsonschema::Registry::new()
        .add("json-schema:///meta/no-validation", &meta_schema)
        .expect("resource should be accepted")
        .add("json-schema:///other", &other)
        .expect("resource should be accepted")
        .prepare()
        .expect("registry builds");
    let runtime = jsonschema::options()
        .with_draft(draft)
        .with_registry(&registry)
        .build(&serde_json::from_str(schema).expect("valid JSON"))
        .expect("valid schema");
    assert_validate_parity(
        is_valid(&instance),
        validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
    assert_eq!(
        evaluation_outputs(&evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

const NESTED_SCHEMA_UNDER_2020_12: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_type_ref":{"$ref":"#/definitions/draft4_type"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft4_exclusive_maximum_ref":{"$ref":"#/definitions/draft4_exclusive_maximum"},"draft201909_prefix_items":{"$schema":"https://json-schema.org/draft/2019-09/schema","prefixItems":[{"type":"string"}]},"draft201909_prefix_items_ref":{"$ref":"#/definitions/draft201909_prefix_items"},"draft7_prefix_items":{"$schema":"http://json-schema.org/draft-07/schema#","prefixItems":[{"type":"string"}]},"draft7_prefix_items_ref":{"$ref":"#/definitions/draft7_prefix_items"},"draft201909_items":{"$schema":"https://json-schema.org/draft/2019-09/schema","items":[{"type":"string"}]},"draft201909_items_ref":{"$ref":"#/definitions/draft201909_items"},"draft7_items":{"$schema":"http://json-schema.org/draft-07/schema#","items":[{"type":"string"}]},"draft7_items_ref":{"$ref":"#/definitions/draft7_items"},"draft7_beside_ref":{"$schema":"http://json-schema.org/draft-07/schema#","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft7_beside_ref_ref":{"$ref":"#/definitions/draft7_beside_ref"}},"definitions":{"any":{},"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft201909_prefix_items":{"$schema":"https://json-schema.org/draft/2019-09/schema","prefixItems":[{"type":"string"}]},"draft7_prefix_items":{"$schema":"http://json-schema.org/draft-07/schema#","prefixItems":[{"type":"string"}]},"draft201909_items":{"$schema":"https://json-schema.org/draft/2019-09/schema","items":[{"type":"string"}]},"draft7_items":{"$schema":"http://json-schema.org/draft-07/schema#","items":[{"type":"string"}]},"draft7_beside_ref":{"$schema":"http://json-schema.org/draft-07/schema#","allOf":[{"$ref":"#/definitions/any","type":"integer"}]}}}"##;
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_type_ref":{"$ref":"#/definitions/draft4_type"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft4_exclusive_maximum_ref":{"$ref":"#/definitions/draft4_exclusive_maximum"},"draft201909_prefix_items":{"$schema":"https://json-schema.org/draft/2019-09/schema","prefixItems":[{"type":"string"}]},"draft201909_prefix_items_ref":{"$ref":"#/definitions/draft201909_prefix_items"},"draft7_prefix_items":{"$schema":"http://json-schema.org/draft-07/schema#","prefixItems":[{"type":"string"}]},"draft7_prefix_items_ref":{"$ref":"#/definitions/draft7_prefix_items"},"draft201909_items":{"$schema":"https://json-schema.org/draft/2019-09/schema","items":[{"type":"string"}]},"draft201909_items_ref":{"$ref":"#/definitions/draft201909_items"},"draft7_items":{"$schema":"http://json-schema.org/draft-07/schema#","items":[{"type":"string"}]},"draft7_items_ref":{"$ref":"#/definitions/draft7_items"},"draft7_beside_ref":{"$schema":"http://json-schema.org/draft-07/schema#","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft7_beside_ref_ref":{"$ref":"#/definitions/draft7_beside_ref"}},"definitions":{"any":{},"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft201909_prefix_items":{"$schema":"https://json-schema.org/draft/2019-09/schema","prefixItems":[{"type":"string"}]},"draft7_prefix_items":{"$schema":"http://json-schema.org/draft-07/schema#","prefixItems":[{"type":"string"}]},"draft201909_items":{"$schema":"https://json-schema.org/draft/2019-09/schema","items":[{"type":"string"}]},"draft7_items":{"$schema":"http://json-schema.org/draft-07/schema#","items":[{"type":"string"}]},"draft7_beside_ref":{"$schema":"http://json-schema.org/draft-07/schema#","allOf":[{"$ref":"#/definitions/any","type":"integer"}]}}}"##
)]
struct NestedSchemaUnder202012;

const NESTED_SCHEMA_UNDER_2019_09: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","properties":{"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_type_ref":{"$ref":"#/definitions/draft4_type"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft4_exclusive_maximum_ref":{"$ref":"#/definitions/draft4_exclusive_maximum"},"draft202012_prefix_items":{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]},"draft202012_prefix_items_ref":{"$ref":"#/definitions/draft202012_prefix_items"},"draft7_beside_ref":{"$schema":"http://json-schema.org/draft-07/schema#","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft7_beside_ref_ref":{"$ref":"#/definitions/draft7_beside_ref"}},"definitions":{"any":{},"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft202012_prefix_items":{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]},"draft7_beside_ref":{"$schema":"http://json-schema.org/draft-07/schema#","allOf":[{"$ref":"#/definitions/any","type":"integer"}]}}}"##;
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","properties":{"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_type_ref":{"$ref":"#/definitions/draft4_type"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft4_exclusive_maximum_ref":{"$ref":"#/definitions/draft4_exclusive_maximum"},"draft202012_prefix_items":{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]},"draft202012_prefix_items_ref":{"$ref":"#/definitions/draft202012_prefix_items"},"draft7_beside_ref":{"$schema":"http://json-schema.org/draft-07/schema#","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft7_beside_ref_ref":{"$ref":"#/definitions/draft7_beside_ref"}},"definitions":{"any":{},"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft202012_prefix_items":{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]},"draft7_beside_ref":{"$schema":"http://json-schema.org/draft-07/schema#","allOf":[{"$ref":"#/definitions/any","type":"integer"}]}}}"##
)]
struct NestedSchemaUnder201909;

const NESTED_SCHEMA_UNDER_DRAFT_7: &str = r##"{"$schema":"http://json-schema.org/draft-07/schema#","properties":{"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_type_ref":{"$ref":"#/definitions/draft4_type"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft4_exclusive_maximum_ref":{"$ref":"#/definitions/draft4_exclusive_maximum"},"draft201909_dependent_required":{"$schema":"https://json-schema.org/draft/2019-09/schema","dependentRequired":{"foo":["bar"]}},"draft201909_dependent_required_ref":{"$ref":"#/definitions/draft201909_dependent_required"},"draft202012_prefix_items":{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]},"draft202012_prefix_items_ref":{"$ref":"#/definitions/draft202012_prefix_items"},"draft201909_beside_ref":{"$schema":"https://json-schema.org/draft/2019-09/schema","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft201909_beside_ref_ref":{"$ref":"#/definitions/draft201909_beside_ref"},"draft202012_beside_ref":{"$schema":"https://json-schema.org/draft/2020-12/schema","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft202012_beside_ref_ref":{"$ref":"#/definitions/draft202012_beside_ref"}},"definitions":{"any":{},"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft201909_dependent_required":{"$schema":"https://json-schema.org/draft/2019-09/schema","dependentRequired":{"foo":["bar"]}},"draft202012_prefix_items":{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]},"draft201909_beside_ref":{"$schema":"https://json-schema.org/draft/2019-09/schema","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft202012_beside_ref":{"$schema":"https://json-schema.org/draft/2020-12/schema","allOf":[{"$ref":"#/definitions/any","type":"integer"}]}}}"##;
#[jsonschema::validator(
    schema = r##"{"$schema":"http://json-schema.org/draft-07/schema#","properties":{"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_type_ref":{"$ref":"#/definitions/draft4_type"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft4_exclusive_maximum_ref":{"$ref":"#/definitions/draft4_exclusive_maximum"},"draft201909_dependent_required":{"$schema":"https://json-schema.org/draft/2019-09/schema","dependentRequired":{"foo":["bar"]}},"draft201909_dependent_required_ref":{"$ref":"#/definitions/draft201909_dependent_required"},"draft202012_prefix_items":{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]},"draft202012_prefix_items_ref":{"$ref":"#/definitions/draft202012_prefix_items"},"draft201909_beside_ref":{"$schema":"https://json-schema.org/draft/2019-09/schema","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft201909_beside_ref_ref":{"$ref":"#/definitions/draft201909_beside_ref"},"draft202012_beside_ref":{"$schema":"https://json-schema.org/draft/2020-12/schema","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft202012_beside_ref_ref":{"$ref":"#/definitions/draft202012_beside_ref"}},"definitions":{"any":{},"draft4_type":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_exclusive_maximum":{"$schema":"http://json-schema.org/draft-04/schema#","maximum":5,"exclusiveMaximum":true},"draft201909_dependent_required":{"$schema":"https://json-schema.org/draft/2019-09/schema","dependentRequired":{"foo":["bar"]}},"draft202012_prefix_items":{"$schema":"https://json-schema.org/draft/2020-12/schema","prefixItems":[{"type":"string"}]},"draft201909_beside_ref":{"$schema":"https://json-schema.org/draft/2019-09/schema","allOf":[{"$ref":"#/definitions/any","type":"integer"}]},"draft202012_beside_ref":{"$schema":"https://json-schema.org/draft/2020-12/schema","allOf":[{"$ref":"#/definitions/any","type":"integer"}]}}}"##
)]
struct NestedSchemaUnderDraft7;

const NESTED_SCHEMA_REFERENCES: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"child":{"$ref":"#/$defs/holder/allOf/0"},"resource":{"$ref":"#/$defs/resource"},"resource_child":{"$ref":"#/$defs/resource/allOf/0"},"inline":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"inline_ref":{"$ref":"#/properties/inline"},"draft4_referrer":{"$schema":"http://json-schema.org/draft-04/schema#","properties":{"root_draft":{"$ref":"#/$defs/root_draft"},"draft4":{"$ref":"#/$defs/draft4"}}},"root_draft":{"$ref":"#/$defs/root_draft"},"draft4":{"$ref":"#/$defs/draft4"},"draft4_id":{"$ref":"#/$defs/draft4_id"}},"$defs":{"holder":{"$schema":"http://json-schema.org/draft-04/schema#","allOf":[{"type":"integer"}]},"resource":{"$id":"https://example.com/resource","id":"https://example.com/resource","$schema":"http://json-schema.org/draft-04/schema#","allOf":[{"type":"integer"}]},"root_draft":{"type":"integer"},"draft4":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_id":{"$schema":"http://json-schema.org/draft-04/schema#","id":"https://example.com/draft4-id","properties":{"n":{"$ref":"#/definitions/y"}},"definitions":{"y":{"type":"integer"}}}}}"##;
// Each reference target is in the draft of where it is written, whatever draft the reference sits in.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"child":{"$ref":"#/$defs/holder/allOf/0"},"resource":{"$ref":"#/$defs/resource"},"resource_child":{"$ref":"#/$defs/resource/allOf/0"},"inline":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"inline_ref":{"$ref":"#/properties/inline"},"draft4_referrer":{"$schema":"http://json-schema.org/draft-04/schema#","properties":{"root_draft":{"$ref":"#/$defs/root_draft"},"draft4":{"$ref":"#/$defs/draft4"}}},"root_draft":{"$ref":"#/$defs/root_draft"},"draft4":{"$ref":"#/$defs/draft4"},"draft4_id":{"$ref":"#/$defs/draft4_id"}},"$defs":{"holder":{"$schema":"http://json-schema.org/draft-04/schema#","allOf":[{"type":"integer"}]},"resource":{"$id":"https://example.com/resource","id":"https://example.com/resource","$schema":"http://json-schema.org/draft-04/schema#","allOf":[{"type":"integer"}]},"root_draft":{"type":"integer"},"draft4":{"$schema":"http://json-schema.org/draft-04/schema#","type":"integer"},"draft4_id":{"$schema":"http://json-schema.org/draft-04/schema#","id":"https://example.com/draft4-id","properties":{"n":{"$ref":"#/definitions/y"}},"definitions":{"y":{"type":"integer"}}}}}"##
)]
struct NestedSchemaReferences;

const NESTED_SCHEMA_IN_EVALUATED_BRANCH: &str = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","anyOf":[{"$schema":"http://json-schema.org/draft-04/schema#","properties":{"a":{}},"allOf":[{"properties":{"a":{"type":"integer"}}}]},{"properties":{"b":true}}],"unevaluatedProperties":false}"#;
// The first branch evaluates `a` only where it accepts, which takes its own draft.
#[jsonschema::validator(
    schema = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","anyOf":[{"$schema":"http://json-schema.org/draft-04/schema#","properties":{"a":{}},"allOf":[{"properties":{"a":{"type":"integer"}}}]},{"properties":{"b":true}}],"unevaluatedProperties":false}"#
)]
struct NestedSchemaInEvaluatedBranch;

// A subschema's `$schema` picks its draft inline and through a `$ref`.
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft4_type": 1.0}), false ; "draft 4 type under 2020-12 inline")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft4_type_ref": 1.0}), false ; "draft 4 type under 2020-12 through ref")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft4_exclusive_maximum": 5}), false ; "draft 4 exclusive maximum under 2020-12 inline")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft4_exclusive_maximum_ref": 5}), false ; "draft 4 exclusive maximum under 2020-12 through ref")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft201909_prefix_items": [1]}), true ; "2019-09 prefix items under 2020-12 inline")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft201909_prefix_items_ref": [1]}), true ; "2019-09 prefix items under 2020-12 through ref")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft7_prefix_items": [1]}), true ; "draft 7 prefix items under 2020-12 inline")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft7_prefix_items_ref": [1]}), true ; "draft 7 prefix items under 2020-12 through ref")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft201909_items": [1]}), false ; "2019-09 items under 2020-12 inline")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft201909_items_ref": [1]}), false ; "2019-09 items under 2020-12 through ref")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft7_items": [1]}), false ; "draft 7 items under 2020-12 inline")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft7_items_ref": [1]}), false ; "draft 7 items under 2020-12 through ref")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft7_beside_ref": "x"}), true ; "draft 7 beside ref under 2020-12 inline")]
#[test_case(NestedSchemaUnder202012::is_valid, NestedSchemaUnder202012::validate, NestedSchemaUnder202012::iter_errors, NestedSchemaUnder202012::evaluate, NESTED_SCHEMA_UNDER_2020_12, serde_json::json!({"draft7_beside_ref_ref": "x"}), true ; "draft 7 beside ref under 2020-12 through ref")]
#[test_case(NestedSchemaUnder201909::is_valid, NestedSchemaUnder201909::validate, NestedSchemaUnder201909::iter_errors, NestedSchemaUnder201909::evaluate, NESTED_SCHEMA_UNDER_2019_09, serde_json::json!({"draft4_type": 1.0}), false ; "draft 4 type under 2019-09 inline")]
#[test_case(NestedSchemaUnder201909::is_valid, NestedSchemaUnder201909::validate, NestedSchemaUnder201909::iter_errors, NestedSchemaUnder201909::evaluate, NESTED_SCHEMA_UNDER_2019_09, serde_json::json!({"draft4_type_ref": 1.0}), false ; "draft 4 type under 2019-09 through ref")]
#[test_case(NestedSchemaUnder201909::is_valid, NestedSchemaUnder201909::validate, NestedSchemaUnder201909::iter_errors, NestedSchemaUnder201909::evaluate, NESTED_SCHEMA_UNDER_2019_09, serde_json::json!({"draft4_exclusive_maximum": 5}), false ; "draft 4 exclusive maximum under 2019-09 inline")]
#[test_case(NestedSchemaUnder201909::is_valid, NestedSchemaUnder201909::validate, NestedSchemaUnder201909::iter_errors, NestedSchemaUnder201909::evaluate, NESTED_SCHEMA_UNDER_2019_09, serde_json::json!({"draft4_exclusive_maximum_ref": 5}), false ; "draft 4 exclusive maximum under 2019-09 through ref")]
#[test_case(NestedSchemaUnder201909::is_valid, NestedSchemaUnder201909::validate, NestedSchemaUnder201909::iter_errors, NestedSchemaUnder201909::evaluate, NESTED_SCHEMA_UNDER_2019_09, serde_json::json!({"draft202012_prefix_items": [1]}), false ; "2020-12 prefix items under 2019-09 inline")]
#[test_case(NestedSchemaUnder201909::is_valid, NestedSchemaUnder201909::validate, NestedSchemaUnder201909::iter_errors, NestedSchemaUnder201909::evaluate, NESTED_SCHEMA_UNDER_2019_09, serde_json::json!({"draft202012_prefix_items_ref": [1]}), false ; "2020-12 prefix items under 2019-09 through ref")]
#[test_case(NestedSchemaUnder201909::is_valid, NestedSchemaUnder201909::validate, NestedSchemaUnder201909::iter_errors, NestedSchemaUnder201909::evaluate, NESTED_SCHEMA_UNDER_2019_09, serde_json::json!({"draft7_beside_ref": "x"}), true ; "draft 7 beside ref under 2019-09 inline")]
#[test_case(NestedSchemaUnder201909::is_valid, NestedSchemaUnder201909::validate, NestedSchemaUnder201909::iter_errors, NestedSchemaUnder201909::evaluate, NESTED_SCHEMA_UNDER_2019_09, serde_json::json!({"draft7_beside_ref_ref": "x"}), true ; "draft 7 beside ref under 2019-09 through ref")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft4_type": 1.0}), false ; "draft 4 type under draft 7 inline")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft4_type_ref": 1.0}), false ; "draft 4 type under draft 7 through ref")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft4_exclusive_maximum": 5}), false ; "draft 4 exclusive maximum under draft 7 inline")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft4_exclusive_maximum_ref": 5}), false ; "draft 4 exclusive maximum under draft 7 through ref")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft201909_dependent_required": {"foo": 1}}), false ; "2019-09 dependent required under draft 7 inline")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft201909_dependent_required_ref": {"foo": 1}}), false ; "2019-09 dependent required under draft 7 through ref")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft202012_prefix_items": [1]}), false ; "2020-12 prefix items under draft 7 inline")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft202012_prefix_items_ref": [1]}), false ; "2020-12 prefix items under draft 7 through ref")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft201909_beside_ref": "x"}), false ; "2019-09 beside ref under draft 7 inline")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft201909_beside_ref_ref": "x"}), false ; "2019-09 beside ref under draft 7 through ref")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft202012_beside_ref": "x"}), false ; "2020-12 beside ref under draft 7 inline")]
#[test_case(NestedSchemaUnderDraft7::is_valid, NestedSchemaUnderDraft7::validate, NestedSchemaUnderDraft7::iter_errors, NestedSchemaUnderDraft7::evaluate, NESTED_SCHEMA_UNDER_DRAFT_7, serde_json::json!({"draft202012_beside_ref_ref": "x"}), false ; "2020-12 beside ref under draft 7 through ref")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"child": 1.0}), false ; "child of a subschema through ref")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"resource": 1.0}), false ; "resource through pointer")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"resource_child": 1.0}), false ; "child of a resource through pointer")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"inline": 1.0}), false ; "subschema inline")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"inline_ref": 1.0}), false ; "subschema inline through ref")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"draft4_referrer": {"root_draft": 1.0}}), true ; "root draft target from draft 4")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"root_draft": 1.0}), true ; "root draft target from root draft")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"draft4_referrer": {"draft4": 1.0}}), false ; "draft 4 target from draft 4")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"draft4": 1.0}), false ; "draft 4 target from root draft")]
#[test_case(NestedSchemaReferences::is_valid, NestedSchemaReferences::validate, NestedSchemaReferences::iter_errors, NestedSchemaReferences::evaluate, NESTED_SCHEMA_REFERENCES, serde_json::json!({"draft4_id": {"n": 1.0}}), false ; "draft 4 id resource through pointer")]
#[test_case(NestedSchemaInEvaluatedBranch::is_valid, NestedSchemaInEvaluatedBranch::validate, NestedSchemaInEvaluatedBranch::iter_errors, NestedSchemaInEvaluatedBranch::evaluate, NESTED_SCHEMA_IN_EVALUATED_BRANCH, serde_json::json!({"a": 1.0, "b": 1}), false ; "branch evaluating in its own draft")]
#[test_case(NestedSchemaInEvaluatedBranch::is_valid, NestedSchemaInEvaluatedBranch::validate, NestedSchemaInEvaluatedBranch::iter_errors, NestedSchemaInEvaluatedBranch::evaluate, NESTED_SCHEMA_IN_EVALUATED_BRANCH, serde_json::json!({"a": 1, "b": 1}), true ; "branch evaluating in its own draft accepts")]
fn test_nested_schema_on_every_path_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
    instance: serde_json::Value,
    expected: bool,
) {
    let runtime = jsonschema::validator_for(&serde_json::from_str(schema).expect("valid JSON"))
        .expect("valid schema");
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_validate_parity(
        is_valid(&instance),
        validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
    assert_eq!(
        evaluation_outputs(&evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

const RECURSIVE_REF_CYCLE_ON_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$recursiveAnchor":true,"minimum":1,"allOf":[{"$recursiveRef":"#"}]}"##;
const RECURSIVE_REF_BESIDE_REF_CYCLE_ON_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$recursiveRef":"#"}}"##;
const RECURSIVE_REF_CYCLE_INTO_NESTED_THEN_SAME_INSTANCE: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"minProperties":1,"allOf":[{"$recursiveRef":"#"}],"additionalProperties":{"$recursiveRef":"#"}}"##;
const RECURSIVE_REF_CYCLE_ON_REF_TARGET: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"tree","$defs":{"tree":{"$id":"tree","$recursiveAnchor":true,"minimum":1,"allOf":[{"$recursiveRef":"#"}]}}}"##;
const DYNAMIC_REF_CYCLE_ON_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"node","minimum":1,"allOf":[{"$dynamicRef":"#node"}]}"##;
const DYNAMIC_REF_BESIDE_REF_CYCLE_ON_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$dynamicRef":"#node"}}"##;
const DYNAMIC_REF_CYCLE_INTO_NESTED_THEN_SAME_INSTANCE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","minProperties":1,"allOf":[{"$dynamicRef":"#node"}],"additionalProperties":{"$dynamicRef":"#node"}}"##;
const DYNAMIC_REF_CYCLE_ON_REF_TARGET: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"tree","$defs":{"tree":{"$id":"tree","$dynamicAnchor":"node","minimum":1,"allOf":[{"$dynamicRef":"#node"}]}}}"##;

const RECURSIVE_REF_NOT_CYCLE_ON_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"not":{"$recursiveRef":"#"}}"##;
const RECURSIVE_REF_NOT_CYCLE_ON_REF_TARGET: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"tree","$defs":{"tree":{"$id":"tree","$recursiveAnchor":true,"not":{"$recursiveRef":"#"}}}}"##;
const DYNAMIC_REF_NOT_CYCLE_ON_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","not":{"$dynamicRef":"#node"}}"##;
const DYNAMIC_REF_NOT_CYCLE_ON_REF_TARGET: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"tree","$defs":{"tree":{"$id":"tree","$dynamicAnchor":"node","not":{"$dynamicRef":"#node"}}}}"##;
// Each `$recursiveRef` or `$dynamicRef` lands on the anchor that holds it, and comes back to the
// instance it holds. Under `not`, validity tells how many times it does.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$recursiveAnchor":true,"minimum":1,"allOf":[{"$recursiveRef":"#"}]}"##,
    draft = Draft201909
)]
struct RecursiveRefCycleOnRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$recursiveRef":"#"}}"##,
    draft = Draft201909
)]
struct RecursiveRefBesideRefCycleOnRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"minProperties":1,"allOf":[{"$recursiveRef":"#"}],"additionalProperties":{"$recursiveRef":"#"}}"##,
    draft = Draft201909
)]
struct RecursiveRefCycleIntoNestedThenSameInstance;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"tree","$defs":{"tree":{"$id":"tree","$recursiveAnchor":true,"minimum":1,"allOf":[{"$recursiveRef":"#"}]}}}"##,
    draft = Draft201909
)]
struct RecursiveRefCycleOnRefTarget;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"node","minimum":1,"allOf":[{"$dynamicRef":"#node"}]}"##
)]
struct DynamicRefCycleOnRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$dynamicRef":"#node"}}"##
)]
struct DynamicRefBesideRefCycleOnRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","minProperties":1,"allOf":[{"$dynamicRef":"#node"}],"additionalProperties":{"$dynamicRef":"#node"}}"##
)]
struct DynamicRefCycleIntoNestedThenSameInstance;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"tree","$defs":{"tree":{"$id":"tree","$dynamicAnchor":"node","minimum":1,"allOf":[{"$dynamicRef":"#node"}]}}}"##
)]
struct DynamicRefCycleOnRefTarget;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"not":{"$recursiveRef":"#"}}"##,
    draft = Draft201909
)]
struct RecursiveRefNotCycleOnRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"tree","$defs":{"tree":{"$id":"tree","$recursiveAnchor":true,"not":{"$recursiveRef":"#"}}}}"##,
    draft = Draft201909
)]
struct RecursiveRefNotCycleOnRefTarget;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","not":{"$dynamicRef":"#node"}}"##
)]
struct DynamicRefNotCycleOnRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"tree","$defs":{"tree":{"$id":"tree","$dynamicAnchor":"node","not":{"$dynamicRef":"#node"}}}}"##
)]
struct DynamicRefNotCycleOnRefTarget;

#[test_case(RecursiveRefCycleOnRoot::is_valid, RecursiveRefCycleOnRoot::validate, RecursiveRefCycleOnRoot::iter_errors, RecursiveRefCycleOnRoot::evaluate, RECURSIVE_REF_CYCLE_ON_ROOT, serde_json::json!(0) ; "recursive_ref_on_root")]
#[test_case(RecursiveRefCycleOnRoot::is_valid, RecursiveRefCycleOnRoot::validate, RecursiveRefCycleOnRoot::iter_errors, RecursiveRefCycleOnRoot::evaluate, RECURSIVE_REF_CYCLE_ON_ROOT, serde_json::json!(1) ; "recursive_ref_on_root_valid")]
#[test_case(RecursiveRefBesideRefCycleOnRoot::is_valid, RecursiveRefBesideRefCycleOnRoot::validate, RecursiveRefBesideRefCycleOnRoot::iter_errors, RecursiveRefBesideRefCycleOnRoot::evaluate, RECURSIVE_REF_BESIDE_REF_CYCLE_ON_ROOT, serde_json::json!({}) ; "recursive_ref_beside_ref_root")]
#[test_case(RecursiveRefBesideRefCycleOnRoot::is_valid, RecursiveRefBesideRefCycleOnRoot::validate, RecursiveRefBesideRefCycleOnRoot::iter_errors, RecursiveRefBesideRefCycleOnRoot::evaluate, RECURSIVE_REF_BESIDE_REF_CYCLE_ON_ROOT, serde_json::json!({"a": {}}) ; "recursive_ref_beside_ref_one_level")]
#[test_case(RecursiveRefBesideRefCycleOnRoot::is_valid, RecursiveRefBesideRefCycleOnRoot::validate, RecursiveRefBesideRefCycleOnRoot::iter_errors, RecursiveRefBesideRefCycleOnRoot::evaluate, RECURSIVE_REF_BESIDE_REF_CYCLE_ON_ROOT, serde_json::json!({"a": {"b": {}}, "c": {}}) ; "recursive_ref_beside_ref_two_levels")]
#[test_case(RecursiveRefCycleIntoNestedThenSameInstance::is_valid, RecursiveRefCycleIntoNestedThenSameInstance::validate, RecursiveRefCycleIntoNestedThenSameInstance::iter_errors, RecursiveRefCycleIntoNestedThenSameInstance::evaluate, RECURSIVE_REF_CYCLE_INTO_NESTED_THEN_SAME_INSTANCE, serde_json::json!({}) ; "recursive_ref_nested_root")]
#[test_case(RecursiveRefCycleIntoNestedThenSameInstance::is_valid, RecursiveRefCycleIntoNestedThenSameInstance::validate, RecursiveRefCycleIntoNestedThenSameInstance::iter_errors, RecursiveRefCycleIntoNestedThenSameInstance::evaluate, RECURSIVE_REF_CYCLE_INTO_NESTED_THEN_SAME_INSTANCE, serde_json::json!({"a": {"b": {}}}) ; "recursive_ref_nested_two_levels")]
#[test_case(RecursiveRefCycleOnRefTarget::is_valid, RecursiveRefCycleOnRefTarget::validate, RecursiveRefCycleOnRefTarget::iter_errors, RecursiveRefCycleOnRefTarget::evaluate, RECURSIVE_REF_CYCLE_ON_REF_TARGET, serde_json::json!(0) ; "recursive_ref_on_ref_target")]
#[test_case(DynamicRefCycleOnRoot::is_valid, DynamicRefCycleOnRoot::validate, DynamicRefCycleOnRoot::iter_errors, DynamicRefCycleOnRoot::evaluate, DYNAMIC_REF_CYCLE_ON_ROOT, serde_json::json!(0) ; "dynamic_ref_on_root")]
#[test_case(DynamicRefCycleOnRoot::is_valid, DynamicRefCycleOnRoot::validate, DynamicRefCycleOnRoot::iter_errors, DynamicRefCycleOnRoot::evaluate, DYNAMIC_REF_CYCLE_ON_ROOT, serde_json::json!(1) ; "dynamic_ref_on_root_valid")]
#[test_case(DynamicRefBesideRefCycleOnRoot::is_valid, DynamicRefBesideRefCycleOnRoot::validate, DynamicRefBesideRefCycleOnRoot::iter_errors, DynamicRefBesideRefCycleOnRoot::evaluate, DYNAMIC_REF_BESIDE_REF_CYCLE_ON_ROOT, serde_json::json!({}) ; "dynamic_ref_beside_ref_root")]
#[test_case(DynamicRefBesideRefCycleOnRoot::is_valid, DynamicRefBesideRefCycleOnRoot::validate, DynamicRefBesideRefCycleOnRoot::iter_errors, DynamicRefBesideRefCycleOnRoot::evaluate, DYNAMIC_REF_BESIDE_REF_CYCLE_ON_ROOT, serde_json::json!({"a": {}}) ; "dynamic_ref_beside_ref_one_level")]
#[test_case(DynamicRefBesideRefCycleOnRoot::is_valid, DynamicRefBesideRefCycleOnRoot::validate, DynamicRefBesideRefCycleOnRoot::iter_errors, DynamicRefBesideRefCycleOnRoot::evaluate, DYNAMIC_REF_BESIDE_REF_CYCLE_ON_ROOT, serde_json::json!({"a": {"b": {}}, "c": {}}) ; "dynamic_ref_beside_ref_two_levels")]
#[test_case(DynamicRefCycleIntoNestedThenSameInstance::is_valid, DynamicRefCycleIntoNestedThenSameInstance::validate, DynamicRefCycleIntoNestedThenSameInstance::iter_errors, DynamicRefCycleIntoNestedThenSameInstance::evaluate, DYNAMIC_REF_CYCLE_INTO_NESTED_THEN_SAME_INSTANCE, serde_json::json!({}) ; "dynamic_ref_nested_root")]
#[test_case(DynamicRefCycleIntoNestedThenSameInstance::is_valid, DynamicRefCycleIntoNestedThenSameInstance::validate, DynamicRefCycleIntoNestedThenSameInstance::iter_errors, DynamicRefCycleIntoNestedThenSameInstance::evaluate, DYNAMIC_REF_CYCLE_INTO_NESTED_THEN_SAME_INSTANCE, serde_json::json!({"a": {"b": {}}}) ; "dynamic_ref_nested_two_levels")]
#[test_case(DynamicRefCycleOnRefTarget::is_valid, DynamicRefCycleOnRefTarget::validate, DynamicRefCycleOnRefTarget::iter_errors, DynamicRefCycleOnRefTarget::evaluate, DYNAMIC_REF_CYCLE_ON_REF_TARGET, serde_json::json!(0) ; "dynamic_ref_on_ref_target")]
#[test_case(RecursiveRefNotCycleOnRoot::is_valid, RecursiveRefNotCycleOnRoot::validate, RecursiveRefNotCycleOnRoot::iter_errors, RecursiveRefNotCycleOnRoot::evaluate, RECURSIVE_REF_NOT_CYCLE_ON_ROOT, serde_json::json!(0) ; "recursive_ref_not_on_root")]
#[test_case(RecursiveRefNotCycleOnRefTarget::is_valid, RecursiveRefNotCycleOnRefTarget::validate, RecursiveRefNotCycleOnRefTarget::iter_errors, RecursiveRefNotCycleOnRefTarget::evaluate, RECURSIVE_REF_NOT_CYCLE_ON_REF_TARGET, serde_json::json!(0) ; "recursive_ref_not_on_ref_target")]
#[test_case(DynamicRefNotCycleOnRoot::is_valid, DynamicRefNotCycleOnRoot::validate, DynamicRefNotCycleOnRoot::iter_errors, DynamicRefNotCycleOnRoot::evaluate, DYNAMIC_REF_NOT_CYCLE_ON_ROOT, serde_json::json!(0) ; "dynamic_ref_not_on_root")]
#[test_case(DynamicRefNotCycleOnRefTarget::is_valid, DynamicRefNotCycleOnRefTarget::validate, DynamicRefNotCycleOnRefTarget::iter_errors, DynamicRefNotCycleOnRefTarget::evaluate, DYNAMIC_REF_NOT_CYCLE_ON_REF_TARGET, serde_json::json!(0) ; "dynamic_ref_not_on_ref_target")]
fn test_dynamic_scope_cycle_back_to_its_instance_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
    instance: serde_json::Value,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        is_valid(&instance),
        validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
    assert_eq!(
        evaluation_outputs(&evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

#[jsonschema::validator(schema = r##"{"not": {"$ref": "#"}}"##, methods = { evaluate = true })]
struct RootNotReferencingItself;

#[jsonschema::validator(
    schema = r##"{"minProperties": 2, "allOf": [{"$ref": "#"}]}"##,
    methods = { evaluate = true }
)]
struct RootAllOfReferencingItself;

#[jsonschema::validator(
    schema = r##"{"minProperties": 2, "allOf": [{"$ref": "#"}]}"##,
    draft = referencing::Draft::Draft4,
    methods = { evaluate = true }
)]
struct RootAllOfReferencingItselfDraft4;

#[jsonschema::validator(
    schema = r##"{"$id": "https://example.com/x", "minProperties": 2, "allOf": [{"$ref": "#"}]}"##,
    draft = referencing::Draft::Draft7,
    methods = { evaluate = true }
)]
struct RootWithIdAllOfReferencingItselfDraft7;

#[jsonschema::validator(
    schema = r##"{"$id": "https://example.com/x", "not": {"$ref": "#"}}"##,
    draft = referencing::Draft::Draft201909,
    methods = { evaluate = true }
)]
struct RootWithIdNotReferencingItselfDraft201909;

#[jsonschema::validator(
    schema = r##"{"type": "object", "properties": {"c": {"$ref": "#"}}, "patternProperties": {"^c": {"$ref": "#"}}, "allOf": [{"$ref": "#"}]}"##,
    methods = { evaluate = true }
)]
struct RootTreeReferencingItself;

#[jsonschema::validator(
    schema = r##"{"type": "object", "properties": {"c": {"$ref": "#"}}, "patternProperties": {"^c": {"$ref": "#"}}, "allOf": [{"$ref": "#"}]}"##,
    draft = referencing::Draft::Draft7,
    methods = { evaluate = true }
)]
struct RootTreeReferencingItselfDraft7;

#[jsonschema::validator(
    schema = r##"{"$dynamicAnchor": "node", "minProperties": 2, "allOf": [{"$ref": "#"}]}"##,
    methods = { evaluate = true }
)]
struct RootWithDynamicAnchorAllOfReferencingItself;

#[jsonschema::validator(
    schema = r##"{"$dynamicAnchor": "node", "not": {"$ref": "#"}}"##,
    methods = { evaluate = true }
)]
struct RootWithDynamicAnchorNotReferencingItself;

#[jsonschema::validator(
    schema = r##"{"$ref": "#", "minimum": 1, "allOf": [{"$ref": "#"}]}"##,
    methods = { evaluate = true }
)]
struct RootBesideReferenceToItself;

// A `$ref: "#"` runs the root's own code, so on a cycle back to the root's instance it stops where
// it would below a root that only references the schema, and one the root holds itself applies
// nothing.
#[test_case(RootNotReferencingItself::is_valid, RootNotReferencingItself::validate, RootNotReferencingItself::iter_errors, RootNotReferencingItself::evaluate, jsonschema::options(), serde_json::json!({"not": {"$ref": "#"}}) ; "not")]
#[test_case(RootAllOfReferencingItself::is_valid, RootAllOfReferencingItself::validate, RootAllOfReferencingItself::iter_errors, RootAllOfReferencingItself::evaluate, jsonschema::options(), serde_json::json!({"minProperties": 2, "allOf": [{"$ref": "#"}]}) ; "all_of")]
#[test_case(RootAllOfReferencingItselfDraft4::is_valid, RootAllOfReferencingItselfDraft4::validate, RootAllOfReferencingItselfDraft4::iter_errors, RootAllOfReferencingItselfDraft4::evaluate, jsonschema::options().with_draft(referencing::Draft::Draft4), serde_json::json!({"minProperties": 2, "allOf": [{"$ref": "#"}]}) ; "all_of_draft4")]
#[test_case(RootWithIdAllOfReferencingItselfDraft7::is_valid, RootWithIdAllOfReferencingItselfDraft7::validate, RootWithIdAllOfReferencingItselfDraft7::iter_errors, RootWithIdAllOfReferencingItselfDraft7::evaluate, jsonschema::options().with_draft(referencing::Draft::Draft7), serde_json::json!({"$id": "https://example.com/x", "minProperties": 2, "allOf": [{"$ref": "#"}]}) ; "all_of_with_id_draft7")]
#[test_case(RootWithIdNotReferencingItselfDraft201909::is_valid, RootWithIdNotReferencingItselfDraft201909::validate, RootWithIdNotReferencingItselfDraft201909::iter_errors, RootWithIdNotReferencingItselfDraft201909::evaluate, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$id": "https://example.com/x", "not": {"$ref": "#"}}) ; "not_with_id_draft2019_09")]
#[test_case(RootTreeReferencingItself::is_valid, RootTreeReferencingItself::validate, RootTreeReferencingItself::iter_errors, RootTreeReferencingItself::evaluate, jsonschema::options(), serde_json::json!({"type": "object", "properties": {"c": {"$ref": "#"}}, "patternProperties": {"^c": {"$ref": "#"}}, "allOf": [{"$ref": "#"}]}) ; "tree")]
#[test_case(RootTreeReferencingItselfDraft7::is_valid, RootTreeReferencingItselfDraft7::validate, RootTreeReferencingItselfDraft7::iter_errors, RootTreeReferencingItselfDraft7::evaluate, jsonschema::options().with_draft(referencing::Draft::Draft7), serde_json::json!({"type": "object", "properties": {"c": {"$ref": "#"}}, "patternProperties": {"^c": {"$ref": "#"}}, "allOf": [{"$ref": "#"}]}) ; "tree_draft7")]
#[test_case(RootWithDynamicAnchorAllOfReferencingItself::is_valid, RootWithDynamicAnchorAllOfReferencingItself::validate, RootWithDynamicAnchorAllOfReferencingItself::iter_errors, RootWithDynamicAnchorAllOfReferencingItself::evaluate, jsonschema::options(), serde_json::json!({"$dynamicAnchor": "node", "minProperties": 2, "allOf": [{"$ref": "#"}]}) ; "all_of_beside_dynamic_anchor")]
#[test_case(RootWithDynamicAnchorNotReferencingItself::is_valid, RootWithDynamicAnchorNotReferencingItself::validate, RootWithDynamicAnchorNotReferencingItself::iter_errors, RootWithDynamicAnchorNotReferencingItself::evaluate, jsonschema::options(), serde_json::json!({"$dynamicAnchor": "node", "not": {"$ref": "#"}}) ; "not_beside_dynamic_anchor")]
#[test_case(RootBesideReferenceToItself::is_valid, RootBesideReferenceToItself::validate, RootBesideReferenceToItself::iter_errors, RootBesideReferenceToItself::evaluate, jsonschema::options(), serde_json::json!({"$ref": "#", "minimum": 1, "allOf": [{"$ref": "#"}]}) ; "beside_reference_to_itself")]
fn test_root_referencing_itself_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    options: jsonschema::ValidationOptions,
    schema: serde_json::Value,
) {
    let runtime = options.build(&schema).expect("valid schema");
    for instance in [
        serde_json::json!({}),
        serde_json::json!({"a": 1, "b": 2}),
        serde_json::json!({"c": {"c": 1}}),
        serde_json::json!({"c": {"c": {}}}),
        serde_json::json!(0),
        serde_json::json!(1),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
            "{instance}"
        );
    }
}

const RECURSIVE_REF_TO_INLINE_RESOURCE_HOLDING_IT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","allOf":[{"$id":"x","properties":{"s":true},"$recursiveRef":"#"}]}"##;
const ANCHORED_RECURSIVE_REF_TO_INLINE_RESOURCE_HOLDING_IT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","allOf":[{"$id":"x","$recursiveAnchor":true,"properties":{"s":true},"$recursiveRef":"#"}]}"##;
const RECURSIVE_REF_TO_ROOT_HOLDING_IT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","properties":{"s":true},"$recursiveRef":"#"}"##;
const RECURSIVE_REF_TO_TARGET_HOLDING_IT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"x","$defs":{"x":{"$id":"x","properties":{"s":true},"$recursiveRef":"#"}}}"##;
const RECURSIVE_REF_HELD_BY_ITS_TARGET_LANDING_ON_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$recursiveAnchor":true,"required":["a"],"properties":{"p":{"$ref":"x"}},"$defs":{"x":{"$id":"x","$recursiveAnchor":true,"$recursiveRef":"#"}}}"##;
const RECURSIVE_REF_HELD_BY_ITS_TARGET_LOOPING_THROUGH_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$recursiveAnchor":true,"properties":{"s":true},"allOf":[{"$ref":"x"}],"$defs":{"x":{"$id":"x","$recursiveAnchor":true,"$recursiveRef":"#"}}}"##;
const REF_TO_INLINE_RESOURCE_HOLDING_IT: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","allOf":[{"$id":"x","properties":{"s":true},"$ref":"#"}]}"##;
const DYNAMIC_REF_TO_INLINE_RESOURCE_HOLDING_IT: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","allOf":[{"$id":"x","$dynamicAnchor":"n","properties":{"s":true},"$dynamicRef":"#n"}]}"##;
const DYNAMIC_REF_HELD_BY_ITS_TARGET_LANDING_ON_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"n","required":["a"],"properties":{"p":{"$ref":"x"}},"$defs":{"x":{"$id":"x","$dynamicAnchor":"n","$dynamicRef":"#n"}}}"##;
const DYNAMIC_REF_HELD_BY_ITS_TARGET_LOOPING_THROUGH_ROOT: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"n","properties":{"s":true},"allOf":[{"$ref":"x"}],"$defs":{"x":{"$id":"x","$dynamicAnchor":"n","$dynamicRef":"#n"}}}"##;
const DYNAMIC_REF_LANDING_ON_ROOT_HOLDING_IT: &str = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"n","properties":{"s":true},"$dynamicRef":"other#n","$defs":{"other":{"$id":"other","$dynamicAnchor":"n","type":"integer"}}}"#;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","allOf":[{"$id":"x","properties":{"s":true},"$recursiveRef":"#"}]}"##,
    draft = Draft201909
)]
struct RecursiveRefToInlineResourceHoldingIt;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","allOf":[{"$id":"x","$recursiveAnchor":true,"properties":{"s":true},"$recursiveRef":"#"}]}"##,
    draft = Draft201909
)]
struct AnchoredRecursiveRefToInlineResourceHoldingIt;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","properties":{"s":true},"$recursiveRef":"#"}"##,
    draft = Draft201909
)]
struct RecursiveRefToRootHoldingIt;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"x","$defs":{"x":{"$id":"x","properties":{"s":true},"$recursiveRef":"#"}}}"##,
    draft = Draft201909
)]
struct RecursiveRefToTargetHoldingIt;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$recursiveAnchor":true,"required":["a"],"properties":{"p":{"$ref":"x"}},"$defs":{"x":{"$id":"x","$recursiveAnchor":true,"$recursiveRef":"#"}}}"##,
    draft = Draft201909
)]
struct RecursiveRefHeldByItsTargetLandingOnRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$recursiveAnchor":true,"properties":{"s":true},"allOf":[{"$ref":"x"}],"$defs":{"x":{"$id":"x","$recursiveAnchor":true,"$recursiveRef":"#"}}}"##,
    draft = Draft201909
)]
struct RecursiveRefHeldByItsTargetLoopingThroughRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","allOf":[{"$id":"x","properties":{"s":true},"$ref":"#"}]}"##
)]
struct RefToInlineResourceHoldingIt;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","allOf":[{"$id":"x","$dynamicAnchor":"n","properties":{"s":true},"$dynamicRef":"#n"}]}"##
)]
struct DynamicRefToInlineResourceHoldingIt;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"n","required":["a"],"properties":{"p":{"$ref":"x"}},"$defs":{"x":{"$id":"x","$dynamicAnchor":"n","$dynamicRef":"#n"}}}"##
)]
struct DynamicRefHeldByItsTargetLandingOnRoot;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"n","properties":{"s":true},"allOf":[{"$ref":"x"}],"$defs":{"x":{"$id":"x","$dynamicAnchor":"n","$dynamicRef":"#n"}}}"##
)]
struct DynamicRefHeldByItsTargetLoopingThroughRoot;

#[jsonschema::validator(
    schema = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicAnchor":"n","properties":{"s":true},"$dynamicRef":"other#n","$defs":{"other":{"$id":"other","$dynamicAnchor":"n","type":"integer"}}}"#
)]
struct DynamicRefLandingOnRootHoldingIt;

// A `$ref` or `$dynamicRef` landing on the schema holding it applies nothing. A `$recursiveRef`
// naming that schema enters it as a reference target, until the cycle guard stops it, and one
// landing elsewhere through the dynamic scope applies the schema it lands on.
#[test_case(RecursiveRefToInlineResourceHoldingIt::is_valid, RecursiveRefToInlineResourceHoldingIt::validate, RecursiveRefToInlineResourceHoldingIt::iter_errors, RecursiveRefToInlineResourceHoldingIt::evaluate, RECURSIVE_REF_TO_INLINE_RESOURCE_HOLDING_IT ; "recursive_ref_in_inline_resource")]
#[test_case(AnchoredRecursiveRefToInlineResourceHoldingIt::is_valid, AnchoredRecursiveRefToInlineResourceHoldingIt::validate, AnchoredRecursiveRefToInlineResourceHoldingIt::iter_errors, AnchoredRecursiveRefToInlineResourceHoldingIt::evaluate, ANCHORED_RECURSIVE_REF_TO_INLINE_RESOURCE_HOLDING_IT ; "anchored_recursive_ref_in_inline_resource")]
#[test_case(RecursiveRefToRootHoldingIt::is_valid, RecursiveRefToRootHoldingIt::validate, RecursiveRefToRootHoldingIt::iter_errors, RecursiveRefToRootHoldingIt::evaluate, RECURSIVE_REF_TO_ROOT_HOLDING_IT ; "recursive_ref_in_root")]
#[test_case(RecursiveRefToTargetHoldingIt::is_valid, RecursiveRefToTargetHoldingIt::validate, RecursiveRefToTargetHoldingIt::iter_errors, RecursiveRefToTargetHoldingIt::evaluate, RECURSIVE_REF_TO_TARGET_HOLDING_IT ; "recursive_ref_in_reference_target")]
#[test_case(RecursiveRefHeldByItsTargetLandingOnRoot::is_valid, RecursiveRefHeldByItsTargetLandingOnRoot::validate, RecursiveRefHeldByItsTargetLandingOnRoot::iter_errors, RecursiveRefHeldByItsTargetLandingOnRoot::evaluate, RECURSIVE_REF_HELD_BY_ITS_TARGET_LANDING_ON_ROOT ; "recursive_ref_landing_on_outer_resource")]
#[test_case(RecursiveRefHeldByItsTargetLoopingThroughRoot::is_valid, RecursiveRefHeldByItsTargetLoopingThroughRoot::validate, RecursiveRefHeldByItsTargetLoopingThroughRoot::iter_errors, RecursiveRefHeldByItsTargetLoopingThroughRoot::evaluate, RECURSIVE_REF_HELD_BY_ITS_TARGET_LOOPING_THROUGH_ROOT ; "recursive_ref_looping_through_outer_resource")]
#[test_case(RefToInlineResourceHoldingIt::is_valid, RefToInlineResourceHoldingIt::validate, RefToInlineResourceHoldingIt::iter_errors, RefToInlineResourceHoldingIt::evaluate, REF_TO_INLINE_RESOURCE_HOLDING_IT ; "ref_in_inline_resource")]
#[test_case(DynamicRefToInlineResourceHoldingIt::is_valid, DynamicRefToInlineResourceHoldingIt::validate, DynamicRefToInlineResourceHoldingIt::iter_errors, DynamicRefToInlineResourceHoldingIt::evaluate, DYNAMIC_REF_TO_INLINE_RESOURCE_HOLDING_IT ; "dynamic_ref_in_inline_resource")]
#[test_case(DynamicRefHeldByItsTargetLandingOnRoot::is_valid, DynamicRefHeldByItsTargetLandingOnRoot::validate, DynamicRefHeldByItsTargetLandingOnRoot::iter_errors, DynamicRefHeldByItsTargetLandingOnRoot::evaluate, DYNAMIC_REF_HELD_BY_ITS_TARGET_LANDING_ON_ROOT ; "dynamic_ref_landing_on_outer_resource")]
#[test_case(DynamicRefHeldByItsTargetLoopingThroughRoot::is_valid, DynamicRefHeldByItsTargetLoopingThroughRoot::validate, DynamicRefHeldByItsTargetLoopingThroughRoot::iter_errors, DynamicRefHeldByItsTargetLoopingThroughRoot::evaluate, DYNAMIC_REF_HELD_BY_ITS_TARGET_LOOPING_THROUGH_ROOT ; "dynamic_ref_looping_through_outer_resource")]
#[test_case(DynamicRefLandingOnRootHoldingIt::is_valid, DynamicRefLandingOnRootHoldingIt::validate, DynamicRefLandingOnRootHoldingIt::iter_errors, DynamicRefLandingOnRootHoldingIt::evaluate, DYNAMIC_REF_LANDING_ON_ROOT_HOLDING_IT ; "dynamic_ref_landing_on_root_holding_it")]
fn test_reference_to_the_schema_holding_it_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for instance in [
        serde_json::json!({"s": 1}),
        serde_json::json!(1),
        serde_json::json!({"a": 1}),
        serde_json::json!({"a": 1, "p": {}}),
        serde_json::json!({"a": 1, "p": {"a": 1}}),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
            "{instance}"
        );
    }
}

const RECURSIVE_REF_IN_EMBEDDED_RESOURCE: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/x","$recursiveAnchor":true,"type":"object","properties":{"c":{"$id":"inner","$recursiveAnchor":true,"properties":{"c":{"$recursiveRef":"#"}}}}}"##;
const RECURSIVE_REF_IN_EMBEDDED_RESOURCE_BEHIND_REF: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$ref":"https://example.com/x","$defs":{"x":{"$id":"https://example.com/x","$recursiveAnchor":true,"type":"object","properties":{"c":{"$id":"inner","$recursiveAnchor":true,"properties":{"c":{"$recursiveRef":"#"}}}}}}}"##;
const DYNAMIC_REF_IN_EMBEDDED_RESOURCE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}"##;
const DYNAMIC_REF_IN_EMBEDDED_RESOURCE_BEHIND_REF: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"https://example.com/x","$defs":{"x":{"$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}}}"##;
const DYNAMIC_REF_PAST_EMBEDDED_RESOURCE_WITHOUT_ANCHOR: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"a":{"$id":"middle","properties":{"b":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}}}"##;
const DYNAMIC_REF_AFTER_EMBEDDED_RESOURCE_AND_REF: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$id":"middle","$ref":"leaf"}},"$defs":{"leaf":{"$id":"leaf","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}"##;
const RECURSIVE_REF_AFTER_POINTER_INTO_EMBEDDED_RESOURCE: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/w","$ref":"x","$defs":{"x":{"$id":"x","$recursiveAnchor":true,"type":"object","properties":{"c":{"$ref":"#/$defs/inner"}},"$defs":{"inner":{"$id":"inner","$recursiveAnchor":true,"properties":{"c":{"$recursiveRef":"#"}}}}}}}"##;
const DYNAMIC_REF_AFTER_POINTER_INTO_EMBEDDED_RESOURCE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/w","$ref":"x","$defs":{"x":{"$id":"x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$ref":"#/$defs/inner"}},"$defs":{"inner":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}}}"##;
const DYNAMIC_REF_IN_EMBEDDED_RESOURCE_UNDER_ITEMS: &str = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"items":{"$id":"inner","$dynamicRef":"middle#node"}}},"$defs":{"middle":{"$id":"middle","$dynamicAnchor":"node"}}}"#;
const REFERENCE_FROM_EMBEDDED_RESOURCE_UNDER_ITEMS: &str = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"items":{"$id":"inner","$ref":"middle"}}},"$defs":{"middle":{"$id":"middle","$dynamicRef":"leaf#node"},"leaf":{"$id":"leaf","$dynamicAnchor":"node"}}}"#;
const DYNAMIC_REF_IN_EMBEDDED_RESOURCE_UNDER_UNEVALUATED_PROPERTIES: &str = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","unevaluatedProperties":{"$id":"inner","$dynamicRef":"middle#node"},"$defs":{"middle":{"$id":"middle","$dynamicAnchor":"node"}}}"#;
const DYNAMIC_REF_IN_EMBEDDED_RESOURCE_UNDER_PROPERTY_NAMES: &str = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","maxLength":1,"propertyNames":{"$id":"inner","$dynamicRef":"middle#node"},"$defs":{"middle":{"$id":"middle","$dynamicAnchor":"node"}}}"#;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/x","$recursiveAnchor":true,"type":"object","properties":{"c":{"$id":"inner","$recursiveAnchor":true,"properties":{"c":{"$recursiveRef":"#"}}}}}"##,
    draft = Draft201909
)]
struct RecursiveRefInEmbeddedResource;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$ref":"https://example.com/x","$defs":{"x":{"$id":"https://example.com/x","$recursiveAnchor":true,"type":"object","properties":{"c":{"$id":"inner","$recursiveAnchor":true,"properties":{"c":{"$recursiveRef":"#"}}}}}}}"##,
    draft = Draft201909
)]
struct RecursiveRefInEmbeddedResourceBehindRef;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}"##
)]
struct DynamicRefInEmbeddedResource;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"https://example.com/x","$defs":{"x":{"$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}}}"##
)]
struct DynamicRefInEmbeddedResourceBehindRef;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"a":{"$id":"middle","properties":{"b":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}}}"##
)]
struct DynamicRefPastEmbeddedResourceWithoutAnchor;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$id":"middle","$ref":"leaf"}},"$defs":{"leaf":{"$id":"leaf","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}"##
)]
struct DynamicRefAfterEmbeddedResourceAndRef;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/w","$ref":"x","$defs":{"x":{"$id":"x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$ref":"#/$defs/inner"}},"$defs":{"inner":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}}}"##
)]
struct DynamicRefAfterPointerIntoEmbeddedResource;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/w","$ref":"x","$defs":{"x":{"$id":"x","$recursiveAnchor":true,"type":"object","properties":{"c":{"$ref":"#/$defs/inner"}},"$defs":{"inner":{"$id":"inner","$recursiveAnchor":true,"properties":{"c":{"$recursiveRef":"#"}}}}}}}"##,
    draft = Draft201909
)]
struct RecursiveRefAfterPointerIntoEmbeddedResource;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"items":{"$id":"inner","$dynamicRef":"middle#node"}}},"$defs":{"middle":{"$id":"middle","$dynamicAnchor":"node"}}}"##
)]
struct DynamicRefInEmbeddedResourceUnderItems;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","maxLength":1,"propertyNames":{"$id":"inner","$dynamicRef":"middle#node"},"$defs":{"middle":{"$id":"middle","$dynamicAnchor":"node"}}}"##
)]
struct DynamicRefInEmbeddedResourceUnderPropertyNames;

#[jsonschema::validator(
    schema = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","properties":{"c":{"items":{"$id":"inner","$ref":"middle"}}},"$defs":{"middle":{"$id":"middle","$dynamicRef":"leaf#node"},"leaf":{"$id":"leaf","$dynamicAnchor":"node"}}}"#
)]
struct ReferenceFromEmbeddedResourceUnderItems;

#[jsonschema::validator(
    schema = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","type":"object","unevaluatedProperties":{"$id":"inner","$dynamicRef":"middle#node"},"$defs":{"middle":{"$id":"middle","$dynamicAnchor":"node"}}}"#
)]
struct DynamicRefInEmbeddedResourceUnderUnevaluatedProperties;

// A resource `x` holds stays in the dynamic scope below it, entered inline or by a pointer, so a
// reference there lands on `x`, the outermost resource with the anchor.
#[test_case(RecursiveRefInEmbeddedResource::is_valid, RecursiveRefInEmbeddedResource::validate, RecursiveRefInEmbeddedResource::iter_errors, RecursiveRefInEmbeddedResource::evaluate, RECURSIVE_REF_IN_EMBEDDED_RESOURCE ; "recursive_ref")]
#[test_case(RecursiveRefInEmbeddedResourceBehindRef::is_valid, RecursiveRefInEmbeddedResourceBehindRef::validate, RecursiveRefInEmbeddedResourceBehindRef::iter_errors, RecursiveRefInEmbeddedResourceBehindRef::evaluate, RECURSIVE_REF_IN_EMBEDDED_RESOURCE_BEHIND_REF ; "recursive_ref_behind_ref")]
#[test_case(DynamicRefInEmbeddedResource::is_valid, DynamicRefInEmbeddedResource::validate, DynamicRefInEmbeddedResource::iter_errors, DynamicRefInEmbeddedResource::evaluate, DYNAMIC_REF_IN_EMBEDDED_RESOURCE ; "dynamic_ref")]
#[test_case(DynamicRefInEmbeddedResourceBehindRef::is_valid, DynamicRefInEmbeddedResourceBehindRef::validate, DynamicRefInEmbeddedResourceBehindRef::iter_errors, DynamicRefInEmbeddedResourceBehindRef::evaluate, DYNAMIC_REF_IN_EMBEDDED_RESOURCE_BEHIND_REF ; "dynamic_ref_behind_ref")]
#[test_case(DynamicRefPastEmbeddedResourceWithoutAnchor::is_valid, DynamicRefPastEmbeddedResourceWithoutAnchor::validate, DynamicRefPastEmbeddedResourceWithoutAnchor::iter_errors, DynamicRefPastEmbeddedResourceWithoutAnchor::evaluate, DYNAMIC_REF_PAST_EMBEDDED_RESOURCE_WITHOUT_ANCHOR ; "dynamic_ref_past_resource_without_anchor")]
#[test_case(DynamicRefInEmbeddedResourceUnderItems::is_valid, DynamicRefInEmbeddedResourceUnderItems::validate, DynamicRefInEmbeddedResourceUnderItems::iter_errors, DynamicRefInEmbeddedResourceUnderItems::evaluate, DYNAMIC_REF_IN_EMBEDDED_RESOURCE_UNDER_ITEMS ; "dynamic_ref_under_items")]
#[test_case(DynamicRefInEmbeddedResourceUnderPropertyNames::is_valid, DynamicRefInEmbeddedResourceUnderPropertyNames::validate, DynamicRefInEmbeddedResourceUnderPropertyNames::iter_errors, DynamicRefInEmbeddedResourceUnderPropertyNames::evaluate, DYNAMIC_REF_IN_EMBEDDED_RESOURCE_UNDER_PROPERTY_NAMES ; "dynamic_ref_under_property_names")]
#[test_case(ReferenceFromEmbeddedResourceUnderItems::is_valid, ReferenceFromEmbeddedResourceUnderItems::validate, ReferenceFromEmbeddedResourceUnderItems::iter_errors, ReferenceFromEmbeddedResourceUnderItems::evaluate, REFERENCE_FROM_EMBEDDED_RESOURCE_UNDER_ITEMS ; "reference_from_resource_under_items")]
#[test_case(DynamicRefInEmbeddedResourceUnderUnevaluatedProperties::is_valid, DynamicRefInEmbeddedResourceUnderUnevaluatedProperties::validate, DynamicRefInEmbeddedResourceUnderUnevaluatedProperties::iter_errors, DynamicRefInEmbeddedResourceUnderUnevaluatedProperties::evaluate, DYNAMIC_REF_IN_EMBEDDED_RESOURCE_UNDER_UNEVALUATED_PROPERTIES ; "dynamic_ref_under_unevaluated_properties")]
#[test_case(RecursiveRefAfterPointerIntoEmbeddedResource::is_valid, RecursiveRefAfterPointerIntoEmbeddedResource::validate, RecursiveRefAfterPointerIntoEmbeddedResource::iter_errors, RecursiveRefAfterPointerIntoEmbeddedResource::evaluate, RECURSIVE_REF_AFTER_POINTER_INTO_EMBEDDED_RESOURCE ; "recursive_ref_after_pointer_into_resource")]
#[test_case(DynamicRefAfterPointerIntoEmbeddedResource::is_valid, DynamicRefAfterPointerIntoEmbeddedResource::validate, DynamicRefAfterPointerIntoEmbeddedResource::iter_errors, DynamicRefAfterPointerIntoEmbeddedResource::evaluate, DYNAMIC_REF_AFTER_POINTER_INTO_EMBEDDED_RESOURCE ; "dynamic_ref_after_pointer_into_resource")]
#[test_case(DynamicRefAfterEmbeddedResourceAndRef::is_valid, DynamicRefAfterEmbeddedResourceAndRef::validate, DynamicRefAfterEmbeddedResourceAndRef::iter_errors, DynamicRefAfterEmbeddedResourceAndRef::evaluate, DYNAMIC_REF_AFTER_EMBEDDED_RESOURCE_AND_REF ; "dynamic_ref_after_resource_and_ref")]
fn test_embedded_resource_in_dynamic_scope_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for instance in [
        serde_json::json!({}),
        serde_json::json!({"c": {"c": 1}}),
        serde_json::json!({"c": {"c": {}}}),
        serde_json::json!({"c": {"c": {"c": 1}}}),
        serde_json::json!({"a": {"b": {"c": 1}}}),
        serde_json::json!({"a": {"b": {"c": {}}}}),
        serde_json::json!({"c": [1]}),
        serde_json::json!({"c": [{}]}),
        serde_json::json!({"ab": 1}),
        serde_json::json!({"a": 1}),
        serde_json::json!(1),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
            "{instance}"
        );
    }
}

const RECURSIVE_REF_PAST_RESOURCE_WITHOUT_ANCHOR: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"anyOf":[{"type":"boolean"},{"$ref":"b"}],"$defs":{"b":{"$id":"https://example.com/b","anyOf":[{"type":"string"},{"$ref":"c"}]},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"anyOf":[{"type":"integer"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}}}"##;
const RECURSIVE_REF_PAST_INLINE_RESOURCE_WITH_ANCHOR_FALSE: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"anyOf":[{"type":"boolean"},{"$id":"https://example.com/b","$recursiveAnchor":false,"anyOf":[{"type":"string"},{"$id":"https://example.com/c","$recursiveAnchor":true,"anyOf":[{"type":"integer"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}]}]}"##;
const RECURSIVE_TARGET_SHARED_BY_TWO_SCOPES_MIDDLE_FIRST: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"anyOf":[{"type":"boolean"},{"type":"object","properties":{"a":{"$ref":"b"},"m":{"$ref":"c"}},"additionalProperties":false}],"$defs":{"b":{"$id":"https://example.com/b","anyOf":[{"type":"string"},{"$ref":"c"}]},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"anyOf":[{"type":"integer"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}}}"##;
const RECURSIVE_REF_PAST_RESOURCE_WITHOUT_ANCHOR_UNEVALUATED_PROPERTIES: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"properties":{"y":true},"$ref":"b","$defs":{"b":{"$id":"https://example.com/b","$ref":"c"},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"properties":{"x":{"$recursiveRef":"#","unevaluatedProperties":false}}}}}"##;
const RECURSIVE_REF_PAST_RESOURCE_WITHOUT_ANCHOR_UNEVALUATED_ITEMS: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"items":[true],"$ref":"b","$defs":{"b":{"$id":"https://example.com/b","$ref":"c"},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"properties":{"x":{"$recursiveRef":"#","unevaluatedItems":false}}}}}"##;
const RECURSIVE_TARGET_SHARED_BY_TWO_SCOPES_MIDDLE_LAST: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"anyOf":[{"type":"boolean"},{"type":"object","properties":{"z":{"$ref":"b"},"m":{"$ref":"c"}},"additionalProperties":false}],"$defs":{"b":{"$id":"https://example.com/b","anyOf":[{"type":"string"},{"$ref":"c"}]},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"anyOf":[{"type":"integer"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}}}"##;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"anyOf":[{"type":"boolean"},{"$ref":"b"}],"$defs":{"b":{"$id":"https://example.com/b","anyOf":[{"type":"string"},{"$ref":"c"}]},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"anyOf":[{"type":"integer"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}}}"##,
    draft = Draft201909
)]
struct RecursiveRefPastResourceWithoutAnchor;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"anyOf":[{"type":"boolean"},{"$id":"https://example.com/b","$recursiveAnchor":false,"anyOf":[{"type":"string"},{"$id":"https://example.com/c","$recursiveAnchor":true,"anyOf":[{"type":"integer"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}]}]}"##,
    draft = Draft201909
)]
struct RecursiveRefPastInlineResourceWithAnchorFalse;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"anyOf":[{"type":"boolean"},{"type":"object","properties":{"a":{"$ref":"b"},"m":{"$ref":"c"}},"additionalProperties":false}],"$defs":{"b":{"$id":"https://example.com/b","anyOf":[{"type":"string"},{"$ref":"c"}]},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"anyOf":[{"type":"integer"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}}}"##,
    draft = Draft201909
)]
struct RecursiveTargetSharedByTwoScopesMiddleFirst;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"anyOf":[{"type":"boolean"},{"type":"object","properties":{"z":{"$ref":"b"},"m":{"$ref":"c"}},"additionalProperties":false}],"$defs":{"b":{"$id":"https://example.com/b","anyOf":[{"type":"string"},{"$ref":"c"}]},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"anyOf":[{"type":"integer"},{"type":"object","additionalProperties":{"$recursiveRef":"#"}}]}}}"##,
    draft = Draft201909
)]
struct RecursiveTargetSharedByTwoScopesMiddleLast;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"properties":{"y":true},"$ref":"b","$defs":{"b":{"$id":"https://example.com/b","$ref":"c"},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"properties":{"x":{"$recursiveRef":"#","unevaluatedProperties":false}}}}}"##,
    draft = Draft201909
)]
struct RecursiveRefPastResourceWithoutAnchorUnevaluatedProperties;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/a","$recursiveAnchor":true,"items":[true],"$ref":"b","$defs":{"b":{"$id":"https://example.com/b","$ref":"c"},"c":{"$id":"https://example.com/c","$recursiveAnchor":true,"properties":{"x":{"$recursiveRef":"#","unevaluatedItems":false}}}}}"##,
    draft = Draft201909
)]
struct RecursiveRefPastResourceWithoutAnchorUnevaluatedItems;

// A `$recursiveRef` in `c` lands on `a`, the outermost resource with `$recursiveAnchor: true`,
// past `b`, which has none.
#[test_case(RecursiveRefPastResourceWithoutAnchor::is_valid, RecursiveRefPastResourceWithoutAnchor::validate, RecursiveRefPastResourceWithoutAnchor::iter_errors, RecursiveRefPastResourceWithoutAnchor::evaluate, RECURSIVE_REF_PAST_RESOURCE_WITHOUT_ANCHOR ; "past_resource_without_anchor")]
#[test_case(RecursiveRefPastInlineResourceWithAnchorFalse::is_valid, RecursiveRefPastInlineResourceWithAnchorFalse::validate, RecursiveRefPastInlineResourceWithAnchorFalse::iter_errors, RecursiveRefPastInlineResourceWithAnchorFalse::evaluate, RECURSIVE_REF_PAST_INLINE_RESOURCE_WITH_ANCHOR_FALSE ; "past_inline_resource_with_anchor_false")]
#[test_case(RecursiveTargetSharedByTwoScopesMiddleFirst::is_valid, RecursiveTargetSharedByTwoScopesMiddleFirst::validate, RecursiveTargetSharedByTwoScopesMiddleFirst::iter_errors, RecursiveTargetSharedByTwoScopesMiddleFirst::evaluate, RECURSIVE_TARGET_SHARED_BY_TWO_SCOPES_MIDDLE_FIRST ; "target_shared_by_two_scopes_middle_first")]
#[test_case(RecursiveTargetSharedByTwoScopesMiddleLast::is_valid, RecursiveTargetSharedByTwoScopesMiddleLast::validate, RecursiveTargetSharedByTwoScopesMiddleLast::iter_errors, RecursiveTargetSharedByTwoScopesMiddleLast::evaluate, RECURSIVE_TARGET_SHARED_BY_TWO_SCOPES_MIDDLE_LAST ; "target_shared_by_two_scopes_middle_last")]
#[test_case(RecursiveRefPastResourceWithoutAnchorUnevaluatedProperties::is_valid, RecursiveRefPastResourceWithoutAnchorUnevaluatedProperties::validate, RecursiveRefPastResourceWithoutAnchorUnevaluatedProperties::iter_errors, RecursiveRefPastResourceWithoutAnchorUnevaluatedProperties::evaluate, RECURSIVE_REF_PAST_RESOURCE_WITHOUT_ANCHOR_UNEVALUATED_PROPERTIES ; "past_resource_without_anchor_unevaluated_properties")]
#[test_case(RecursiveRefPastResourceWithoutAnchorUnevaluatedItems::is_valid, RecursiveRefPastResourceWithoutAnchorUnevaluatedItems::validate, RecursiveRefPastResourceWithoutAnchorUnevaluatedItems::iter_errors, RecursiveRefPastResourceWithoutAnchorUnevaluatedItems::evaluate, RECURSIVE_REF_PAST_RESOURCE_WITHOUT_ANCHOR_UNEVALUATED_ITEMS ; "past_resource_without_anchor_unevaluated_items")]
fn test_recursive_ref_past_resource_without_anchor_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for instance in [
        serde_json::json!({"x": true}),
        serde_json::json!({"x": "s"}),
        serde_json::json!({"x": 1}),
        serde_json::json!({"x": {"y": true}}),
        serde_json::json!({"x": []}),
        serde_json::json!({"m": {"x": true}}),
        serde_json::json!({"m": {"x": 1}}),
        serde_json::json!({"a": {"x": true}}),
        serde_json::json!({"a": {"x": 1}}),
        serde_json::json!({"z": {"x": true}}),
        serde_json::json!({"z": {"x": 1}}),
        serde_json::json!({"x": {"y": 1}}),
        serde_json::json!({"x": {"z": 1}}),
        serde_json::json!({"x": [1]}),
        serde_json::json!({"x": [1, 2]}),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
            "{instance}"
        );
    }
}

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","properties":{"c":{"$id":"inner","$dynamicAnchor":"node","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"number"}},"properties":{"c":{"$dynamicRef":"#node"}}},"d":{"$dynamicRef":"leaf#leaf"}},"$defs":{"leaf":{"$id":"leaf","$dynamicAnchor":"leaf","type":"string"}}}"##
)]
struct DynamicRefLandingOutsideItsResource;

// A `$dynamicRef` in `inner` lands on `x`, which it enters as a reference does, not as a resource
// held inline below `inner`.
#[test_case(serde_json::json!({"c": {"c": {"d": 1}}}) ; "number")]
#[test_case(serde_json::json!({"c": {"c": {"d": "s"}}}) ; "string")]
#[test_case(serde_json::json!({"d": 1}) ; "outside inner")]
fn test_dynamic_ref_landing_outside_its_resource_matches_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/x", "$dynamicAnchor": "node", "properties": {"c": {"$id": "inner", "$dynamicAnchor": "node", "$defs": {"leaf": {"$dynamicAnchor": "leaf", "type": "number"}}, "properties": {"c": {"$dynamicRef": "#node"}}}, "d": {"$dynamicRef": "leaf#leaf"}}, "$defs": {"leaf": {"$id": "leaf", "$dynamicAnchor": "leaf", "type": "string"}}});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        DynamicRefLandingOutsideItsResource::is_valid(&instance),
        DynamicRefLandingOutsideItsResource::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        DynamicRefLandingOutsideItsResource::iter_errors(&instance),
        &runtime,
        &instance,
    );
    assert_eq!(
        evaluation_outputs(&DynamicRefLandingOutsideItsResource::evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

const DYNAMIC_REF_LANDING_OUTSIDE_ITS_RESOURCE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/x","$dynamicAnchor":"node","properties":{"c":{"$id":"inner","$dynamicAnchor":"node","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"number"}},"properties":{"c":{"$dynamicRef":"#node"}}},"d":{"$dynamicRef":"leaf#leaf"}},"$defs":{"leaf":{"$id":"leaf","$dynamicAnchor":"leaf","type":"string"}}}"##;
const CALLER_SCOPE_THROUGH_REF: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","type":"string"}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","type":"integer"}}}}}"##;
const CALLER_SCOPE_THROUGH_DYNAMIC_REF: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","$defs":{"a":{"$dynamicAnchor":"a","$dynamicRef":"leaf#b"},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","type":"string"}}},"leaf":{"$id":"leaf","$defs":{"b":{"$dynamicAnchor":"b","type":"integer"}}}}}"##;
const CALLER_SCOPE_INTERMEDIATE_RESOURCE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"middle","$defs":{"middle":{"$id":"middle","$ref":"caller","$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"}}},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","type":"string"}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","type":"integer"}}}}}"##;
const CALLER_SCOPE_ANOTHER_RESOURCE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"caller":{"$id":"caller","$dynamicRef":"anchors#a","$defs":{"b":{"$dynamicAnchor":"b","type":"string"}}},"anchors":{"$id":"anchors","$defs":{"a":{"$dynamicAnchor":"a","type":"null"}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","type":"integer"}}}}}"##;
const CALLER_SCOPE_UNEVALUATED_PROPERTIES: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","unevaluatedProperties":false,"$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","properties":{"s":true}}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","properties":{"i":true}}}}}}"##;
const CALLER_SCOPE_UNEVALUATED_ITEMS: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","unevaluatedItems":false,"$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","prefixItems":[true]}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","prefixItems":[true,true]}}}}}"##;
const CALLER_SCOPE_UNEVALUATED_PROPERTIES_INLINE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","unevaluatedProperties":false,"allOf":[{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","properties":{"s":true}}}}],"$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","properties":{"i":true}}}}}}"##;
const CALLER_SCOPE_UNEVALUATED_ITEMS_INLINE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","unevaluatedItems":false,"allOf":[{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","prefixItems":[true]}}}],"$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","prefixItems":[true,true]}}}}}"##;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","type":"string"}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","type":"integer"}}}}}"##
)]
struct CallerScopeThroughRef;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","$defs":{"a":{"$dynamicAnchor":"a","$dynamicRef":"leaf#b"},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","type":"string"}}},"leaf":{"$id":"leaf","$defs":{"b":{"$dynamicAnchor":"b","type":"integer"}}}}}"##
)]
struct CallerScopeThroughDynamicRef;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"middle","$defs":{"middle":{"$id":"middle","$ref":"caller","$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"}}},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","type":"string"}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","type":"integer"}}}}}"##
)]
struct CallerScopeIntermediateResource;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"caller":{"$id":"caller","$dynamicRef":"anchors#a","$defs":{"b":{"$dynamicAnchor":"b","type":"string"}}},"anchors":{"$id":"anchors","$defs":{"a":{"$dynamicAnchor":"a","type":"null"}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","type":"integer"}}}}}"##
)]
struct CallerScopeAnotherResource;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","unevaluatedProperties":false,"$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","properties":{"s":true}}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","properties":{"i":true}}}}}}"##
)]
struct CallerScopeUnevaluatedProperties;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"caller","unevaluatedItems":false,"$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"caller":{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","prefixItems":[true]}}},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","prefixItems":[true,true]}}}}}"##
)]
struct CallerScopeUnevaluatedItems;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","unevaluatedProperties":false,"allOf":[{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","properties":{"s":true}}}}],"$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","properties":{"i":true}}}}}}"##
)]
struct CallerScopeUnevaluatedPropertiesInline;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","unevaluatedItems":false,"allOf":[{"$id":"caller","$dynamicRef":"#a","$defs":{"a":{"$dynamicAnchor":"a","type":"null"},"b":{"$dynamicAnchor":"b","prefixItems":[true]}}}],"$defs":{"a":{"$dynamicAnchor":"a","$ref":"leaf"},"leaf":{"$id":"leaf","$dynamicRef":"#b","$defs":{"b":{"$dynamicAnchor":"b","prefixItems":[true,true]}}}}}"##
)]
struct CallerScopeUnevaluatedItemsInline;

// `caller` declares `b`, which no resource around it does. Its `$dynamicRef` lands in another
// resource, below which `caller` stays in the dynamic scope, so `leaf`'s `$dynamicRef "#b"` binds
// `caller`'s `b` rather than its own.
#[test_case(CallerScopeThroughRef::is_valid, CallerScopeThroughRef::validate, CallerScopeThroughRef::iter_errors, CallerScopeThroughRef::evaluate, CALLER_SCOPE_THROUGH_REF, serde_json::json!("x"), serde_json::json!(1) ; "landed target holds a reference")]
#[test_case(CallerScopeThroughDynamicRef::is_valid, CallerScopeThroughDynamicRef::validate, CallerScopeThroughDynamicRef::iter_errors, CallerScopeThroughDynamicRef::evaluate, CALLER_SCOPE_THROUGH_DYNAMIC_REF, serde_json::json!("x"), serde_json::json!(1) ; "landed target holds a dynamic reference")]
#[test_case(CallerScopeIntermediateResource::is_valid, CallerScopeIntermediateResource::validate, CallerScopeIntermediateResource::iter_errors, CallerScopeIntermediateResource::evaluate, CALLER_SCOPE_INTERMEDIATE_RESOURCE, serde_json::json!("x"), serde_json::json!(1) ; "lands in an intermediate resource")]
#[test_case(CallerScopeAnotherResource::is_valid, CallerScopeAnotherResource::validate, CallerScopeAnotherResource::iter_errors, CallerScopeAnotherResource::evaluate, CALLER_SCOPE_ANOTHER_RESOURCE, serde_json::json!("x"), serde_json::json!(1) ; "initial target in another resource")]
#[test_case(DynamicRefLandingOutsideItsResource::is_valid, DynamicRefLandingOutsideItsResource::validate, DynamicRefLandingOutsideItsResource::iter_errors, DynamicRefLandingOutsideItsResource::evaluate, DYNAMIC_REF_LANDING_OUTSIDE_ITS_RESOURCE, serde_json::json!({"c": {"c": {"d": 1}}}), serde_json::json!({"c": {"c": {"d": "s"}}}) ; "caller entered inline")]
#[test_case(CallerScopeUnevaluatedProperties::is_valid, CallerScopeUnevaluatedProperties::validate, CallerScopeUnevaluatedProperties::iter_errors, CallerScopeUnevaluatedProperties::evaluate, CALLER_SCOPE_UNEVALUATED_PROPERTIES, serde_json::json!({"s": 1}), serde_json::json!({"i": 1}) ; "unevaluated properties")]
#[test_case(CallerScopeUnevaluatedItems::is_valid, CallerScopeUnevaluatedItems::validate, CallerScopeUnevaluatedItems::iter_errors, CallerScopeUnevaluatedItems::evaluate, CALLER_SCOPE_UNEVALUATED_ITEMS, serde_json::json!([1]), serde_json::json!([1, 2]) ; "unevaluated items")]
#[test_case(CallerScopeUnevaluatedPropertiesInline::is_valid, CallerScopeUnevaluatedPropertiesInline::validate, CallerScopeUnevaluatedPropertiesInline::iter_errors, CallerScopeUnevaluatedPropertiesInline::evaluate, CALLER_SCOPE_UNEVALUATED_PROPERTIES_INLINE, serde_json::json!({"s": 1}), serde_json::json!({"i": 1}) ; "unevaluated properties, caller entered inline")]
#[test_case(CallerScopeUnevaluatedItemsInline::is_valid, CallerScopeUnevaluatedItemsInline::validate, CallerScopeUnevaluatedItemsInline::iter_errors, CallerScopeUnevaluatedItemsInline::evaluate, CALLER_SCOPE_UNEVALUATED_ITEMS_INLINE, serde_json::json!([1]), serde_json::json!([1, 2]) ; "unevaluated items, caller entered inline")]
fn test_dynamic_ref_keeps_its_resource_in_dynamic_scope_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
    valid: serde_json::Value,
    invalid: serde_json::Value,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for (instance, expected) in [(valid, true), (invalid, false)] {
        assert_eq!(is_valid(&instance), expected, "{instance}");
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
            "{instance}"
        );
    }
}

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/w","properties":{"d":{"$ref":"inner"}},"$defs":{"x":{"$id":"x","$dynamicAnchor":"node","type":"object","properties":{"c":{"$id":"inner","$dynamicAnchor":"node","properties":{"c":{"$dynamicRef":"#node"}}}}}}}"##
)]
struct EmbeddedResourceReachedFromOutsideItsHolder;

// A `$ref` from outside `x` to a resource `x` holds inline leaves `x` out of the dynamic scope.
#[test_case(serde_json::json!({"d": {"c": 1}}) ; "number")]
#[test_case(serde_json::json!({"d": {"c": {"c": 1}}}) ; "nested")]
fn test_embedded_resource_reached_from_outside_its_holder_matches_runtime(
    instance: serde_json::Value,
) {
    let schema = serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/w", "properties": {"d": {"$ref": "inner"}}, "$defs": {"x": {"$id": "x", "$dynamicAnchor": "node", "type": "object", "properties": {"c": {"$id": "inner", "$dynamicAnchor": "node", "properties": {"c": {"$dynamicRef": "#node"}}}}}}});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        EmbeddedResourceReachedFromOutsideItsHolder::is_valid(&instance),
        EmbeddedResourceReachedFromOutsideItsHolder::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        EmbeddedResourceReachedFromOutsideItsHolder::iter_errors(&instance),
        &runtime,
        &instance,
    );
    assert_eq!(
        evaluation_outputs(&EmbeddedResourceReachedFromOutsideItsHolder::evaluate(
            &instance
        )),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

const DYNAMIC_SCOPE_NESTED_THEN_SAME_INSTANCE: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","type":"object","allOf":[{"$ref":"#"}],"additionalProperties":{"$dynamicRef":"#node"}}"##;
const DYNAMIC_SCOPE_TWO_PATHS: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","type":"object","properties":{"c":{"$dynamicRef":"#node"}},"patternProperties":{"^c":{"$dynamicRef":"#node"}}}"##;
const RECURSIVE_SCOPE_TWO_PATHS: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"type":"object","properties":{"c":{"$recursiveRef":"#"}},"patternProperties":{"^c":{"$recursiveRef":"#"}}}"##;

// Under a dynamic scope, each cycle reaches every nested instance more than once.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","type":"object","allOf":[{"$ref":"#"}],"additionalProperties":{"$dynamicRef":"#node"}}"##
)]
struct DynamicScopeNestedThenSameInstance;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$dynamicAnchor":"node","type":"object","properties":{"c":{"$dynamicRef":"#node"}},"patternProperties":{"^c":{"$dynamicRef":"#node"}}}"##
)]
struct DynamicScopeTwoPaths;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"type":"object","properties":{"c":{"$recursiveRef":"#"}},"patternProperties":{"^c":{"$recursiveRef":"#"}}}"##,
    draft = Draft201909
)]
struct RecursiveScopeTwoPaths;

#[test_case(DynamicScopeNestedThenSameInstance::is_valid, DynamicScopeNestedThenSameInstance::validate, DynamicScopeNestedThenSameInstance::iter_errors, DYNAMIC_SCOPE_NESTED_THEN_SAME_INSTANCE, serde_json::json!({"c": {"c": {}}}) ; "dynamic_nested_then_same_instance_valid")]
#[test_case(DynamicScopeNestedThenSameInstance::is_valid, DynamicScopeNestedThenSameInstance::validate, DynamicScopeNestedThenSameInstance::iter_errors, DYNAMIC_SCOPE_NESTED_THEN_SAME_INSTANCE, serde_json::json!({"c": {"c": 1}}) ; "dynamic_nested_then_same_instance_invalid")]
#[test_case(DynamicScopeTwoPaths::is_valid, DynamicScopeTwoPaths::validate, DynamicScopeTwoPaths::iter_errors, DYNAMIC_SCOPE_TWO_PATHS, serde_json::json!({"c": {"c": {}}}) ; "dynamic_two_paths_valid")]
#[test_case(DynamicScopeTwoPaths::is_valid, DynamicScopeTwoPaths::validate, DynamicScopeTwoPaths::iter_errors, DYNAMIC_SCOPE_TWO_PATHS, serde_json::json!({"c": {"c": 1}}) ; "dynamic_two_paths_invalid")]
#[test_case(RecursiveScopeTwoPaths::is_valid, RecursiveScopeTwoPaths::validate, RecursiveScopeTwoPaths::iter_errors, RECURSIVE_SCOPE_TWO_PATHS, serde_json::json!({"c": {"c": {}}}) ; "recursive_two_paths_valid")]
#[test_case(RecursiveScopeTwoPaths::is_valid, RecursiveScopeTwoPaths::validate, RecursiveScopeTwoPaths::iter_errors, RECURSIVE_SCOPE_TWO_PATHS, serde_json::json!({"c": {"c": 1}}) ; "recursive_two_paths_invalid")]
fn test_ref_cycle_reaching_instances_twice_under_dynamic_scope_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    schema: &str,
    instance: serde_json::Value,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        is_valid(&instance),
        validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
}

// Without the results kept for the run, the work grows by a constant factor at every level.
#[test_case(DynamicScopeNestedThenSameInstance::is_valid ; "dynamic_nested_then_same_instance")]
#[test_case(DynamicScopeTwoPaths::is_valid ; "dynamic_two_paths")]
#[test_case(RecursiveScopeTwoPaths::is_valid ; "recursive_two_paths")]
fn test_ref_cycle_reaching_instances_twice_under_dynamic_scope_keeps_results(is_valid: IsValidFn) {
    assert!(is_valid(&chain_of_c(64)));
}

// `tree` runs on each instance under scopes binding `leaf` to strings and to numbers. A result is
// kept once it costs enough, which an accepted instance does, and answers for its scope alone.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","oneOf":[{"$ref":"strings"},{"$ref":"numbers"}],"$defs":{"strings":{"$id":"strings","$ref":"tree","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"string"}}},"numbers":{"$id":"numbers","$ref":"tree","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"number"}}},"tree":{"$id":"tree","properties":{"c":{"$ref":"#"}},"patternProperties":{"^c":{"$ref":"#"}},"additionalProperties":{"$dynamicRef":"#leaf"},"$defs":{"leaf":{"$dynamicAnchor":"leaf"}}}}}"##
)]
struct DynamicAnchorBoundToStringsThenNumbers;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","oneOf":[{"$ref":"numbers"},{"$ref":"strings"}],"$defs":{"strings":{"$id":"strings","$ref":"tree","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"string"}}},"numbers":{"$id":"numbers","$ref":"tree","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"number"}}},"tree":{"$id":"tree","properties":{"c":{"$ref":"#"}},"patternProperties":{"^c":{"$ref":"#"}},"additionalProperties":{"$dynamicRef":"#leaf"},"$defs":{"leaf":{"$dynamicAnchor":"leaf"}}}}}"##
)]
struct DynamicAnchorBoundToNumbersThenStrings;

const DYNAMIC_ANCHOR_BOUND_TO_STRINGS_THEN_NUMBERS: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","oneOf":[{"$ref":"strings"},{"$ref":"numbers"}],"$defs":{"strings":{"$id":"strings","$ref":"tree","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"string"}}},"numbers":{"$id":"numbers","$ref":"tree","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"number"}}},"tree":{"$id":"tree","properties":{"c":{"$ref":"#"}},"patternProperties":{"^c":{"$ref":"#"}},"additionalProperties":{"$dynamicRef":"#leaf"},"$defs":{"leaf":{"$dynamicAnchor":"leaf"}}}}}"##;
const DYNAMIC_ANCHOR_BOUND_TO_NUMBERS_THEN_STRINGS: &str = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","oneOf":[{"$ref":"numbers"},{"$ref":"strings"}],"$defs":{"strings":{"$id":"strings","$ref":"tree","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"string"}}},"numbers":{"$id":"numbers","$ref":"tree","$defs":{"leaf":{"$dynamicAnchor":"leaf","type":"number"}}},"tree":{"$id":"tree","properties":{"c":{"$ref":"#"}},"patternProperties":{"^c":{"$ref":"#"}},"additionalProperties":{"$dynamicRef":"#leaf"},"$defs":{"leaf":{"$dynamicAnchor":"leaf"}}}}}"##;

// Each level holds the next one as `c`, the deepest `{"x": leaf}`.
fn chain_of_c_to(depth: usize, leaf: serde_json::Value) -> serde_json::Value {
    (0..depth).fold(
        serde_json::json!({"x": leaf}),
        |child, _| serde_json::json!({"c": child}),
    )
}

#[test_case(DynamicAnchorBoundToStringsThenNumbers::is_valid, DynamicAnchorBoundToStringsThenNumbers::validate, DynamicAnchorBoundToStringsThenNumbers::iter_errors, DYNAMIC_ANCHOR_BOUND_TO_STRINGS_THEN_NUMBERS, serde_json::json!(1), true ; "strings_then_numbers_number")]
#[test_case(DynamicAnchorBoundToStringsThenNumbers::is_valid, DynamicAnchorBoundToStringsThenNumbers::validate, DynamicAnchorBoundToStringsThenNumbers::iter_errors, DYNAMIC_ANCHOR_BOUND_TO_STRINGS_THEN_NUMBERS, serde_json::json!("a"), true ; "strings_then_numbers_string")]
#[test_case(DynamicAnchorBoundToStringsThenNumbers::is_valid, DynamicAnchorBoundToStringsThenNumbers::validate, DynamicAnchorBoundToStringsThenNumbers::iter_errors, DYNAMIC_ANCHOR_BOUND_TO_STRINGS_THEN_NUMBERS, serde_json::json!(null), false ; "strings_then_numbers_null")]
#[test_case(DynamicAnchorBoundToNumbersThenStrings::is_valid, DynamicAnchorBoundToNumbersThenStrings::validate, DynamicAnchorBoundToNumbersThenStrings::iter_errors, DYNAMIC_ANCHOR_BOUND_TO_NUMBERS_THEN_STRINGS, serde_json::json!(1), true ; "numbers_then_strings_number")]
#[test_case(DynamicAnchorBoundToNumbersThenStrings::is_valid, DynamicAnchorBoundToNumbersThenStrings::validate, DynamicAnchorBoundToNumbersThenStrings::iter_errors, DYNAMIC_ANCHOR_BOUND_TO_NUMBERS_THEN_STRINGS, serde_json::json!("a"), true ; "numbers_then_strings_string")]
fn test_ref_cycle_results_are_kept_per_dynamic_scope(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    schema: &str,
    leaf: serde_json::Value,
    expected: bool,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    let instance = chain_of_c_to(8, leaf);
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_validate_parity(
        is_valid(&instance),
        validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
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

#[jsonschema::validator(
    schema = r#"{"contentEncoding":"base64","contentMediaType":"application/json"}"#,
    draft = Draft7
)]
struct Base64JsonContentValidator;

// `"abcd"` decodes to bytes that are not UTF-8.
#[test_case(serde_json::json!("abcd") ; "not_utf8")]
#[test_case(serde_json::json!("!!") ; "not_base64")]
#[test_case(serde_json::json!("ew==") ; "not_json")]
#[test_case(serde_json::json!("MQ==") ; "valid")]
fn test_base64_json_content_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(&serde_json::json!({"contentEncoding": "base64", "contentMediaType": "application/json"}))
        .expect("valid schema");
    assert_validate_parity(
        Base64JsonContentValidator::is_valid(&instance),
        Base64JsonContentValidator::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        Base64JsonContentValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"contentEncoding":"base64","contentMediaType":"text/x-unknown"}"#,
    draft = Draft7
)]
struct Base64UnknownMediaTypeContentValidator;

// An unknown media type leaves the encoding checked on its own.
#[test_case(serde_json::json!("!!") ; "not_base64")]
#[test_case(serde_json::json!("ew==") ; "valid")]
fn test_base64_unknown_media_type_content_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .build(
            &serde_json::json!({"contentEncoding": "base64", "contentMediaType": "text/x-unknown"}),
        )
        .expect("valid schema");
    assert_validate_parity(
        Base64UnknownMediaTypeContentValidator::is_valid(&instance),
        Base64UnknownMediaTypeContentValidator::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        Base64UnknownMediaTypeContentValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

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

    let schema = serde_json::json!({"type":"string","pattern":"^abc"});
    let instance = serde_json::json!("xyz");
    assert_validate_parity_for(
        &schema,
        PatternPrefixParityValidator::is_valid(&instance),
        PatternPrefixParityValidator::validate(&instance),
        &instance,
    );
}

// The engine would unroll a huge count past the default size limit; the runtime counts characters
// or collapses the count instead, and so does generated code.
#[jsonschema::validator(schema = r#"{"pattern":"^.{0,262144}$"}"#)]
struct HugeCountPatternValidator;

#[jsonschema::validator(
    schema = r#"{"pattern":"^.{0,262144}$"}"#,
    pattern_options = {
        engine = regex
    }
)]
struct HugeCountPatternRegexValidator;

#[jsonschema::validator(schema = r#"{"pattern":"[^\\p{C}]{1,204800}"}"#)]
struct HugeEdgeCountPatternValidator;

#[jsonschema::validator(
    schema = r#"{"patternProperties":{"a+":{},"^[^\\p{C}]{1,204800}$":{"type":"string"}}}"#
)]
struct HugeCountPatternPropertiesValidator;

#[test_case(serde_json::json!("ab") ; "matches")]
#[test_case(serde_json::json!("a\nb") ; "line feed")]
#[test_case(serde_json::json!("\u{1}") ; "control character")]
#[test_case(serde_json::json!(42) ; "not a string")]
fn test_huge_count_pattern_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"pattern":"^.{0,262144}$"});
    assert_validate_parity_for(
        &schema,
        HugeCountPatternValidator::is_valid(&instance),
        HugeCountPatternValidator::validate(&instance),
        &instance,
    );
    let runtime = jsonschema::options()
        .with_pattern_options(jsonschema::PatternOptions::regex())
        .build(&schema)
        .expect("valid schema");
    assert_validate_parity(
        HugeCountPatternRegexValidator::is_valid(&instance),
        HugeCountPatternRegexValidator::validate(&instance),
        &runtime,
        &instance,
    );
    let schema = serde_json::json!({"pattern":"[^\\p{C}]{1,204800}"});
    assert_validate_parity_for(
        &schema,
        HugeEdgeCountPatternValidator::is_valid(&instance),
        HugeEdgeCountPatternValidator::validate(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!({"ab": 1}) ; "matching key")]
#[test_case(serde_json::json!({"ab": "x"}) ; "matching key with a string")]
#[test_case(serde_json::json!({"\u{1}": 1}) ; "control character key")]
fn test_huge_count_pattern_properties_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"patternProperties":{"a+":{},"^[^\\p{C}]{1,204800}$":{"type":"string"}}});
    assert_validate_parity_for(
        &schema,
        HugeCountPatternPropertiesValidator::is_valid(&instance),
        HugeCountPatternPropertiesValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(schema = r#"{"pattern":"^(?<n>\\d)\\k<n>$"}"#)]
struct NamedBackreferencePatternValidator;

#[jsonschema::validator(schema = r#"{"pattern":"^(a)?\\1b$"}"#)]
struct UnmatchedGroupBackreferencePatternValidator;

#[jsonschema::validator(
    schema = r#"{"patternProperties":{"^(?<n>.)\\k<n>$":false},"propertyNames":{"pattern":"^(?<n>.)\\k<n>"}}"#
)]
struct NamedBackreferenceKeysValidator;

#[test_case(serde_json::json!("33") ; "repeated digit")]
#[test_case(serde_json::json!("34") ; "different digits")]
#[test_case(serde_json::json!("٣٣") ; "repeated non-ascii digit")]
#[test_case(serde_json::json!("b") ; "unmatched group")]
#[test_case(serde_json::json!("ab") ; "matched group without a repeat")]
fn test_backreference_pattern_parity(instance: serde_json::Value) {
    assert_validate_parity_for(
        &serde_json::json!({"pattern":"^(?<n>\\d)\\k<n>$"}),
        NamedBackreferencePatternValidator::is_valid(&instance),
        NamedBackreferencePatternValidator::validate(&instance),
        &instance,
    );
    assert_validate_parity_for(
        &serde_json::json!({"pattern":"^(a)?\\1b$"}),
        UnmatchedGroupBackreferencePatternValidator::is_valid(&instance),
        UnmatchedGroupBackreferencePatternValidator::validate(&instance),
        &instance,
    );
}

#[test_case(serde_json::json!({"aa": 1}) ; "key the pattern map forbids")]
#[test_case(serde_json::json!({"aab": 1}) ; "allowed key")]
#[test_case(serde_json::json!({"ab": 1}) ; "key without a repeat")]
fn test_backreference_keys_parity(instance: serde_json::Value) {
    assert_validate_parity_for(
        &serde_json::json!({"patternProperties":{"^(?<n>.)\\k<n>$":false},"propertyNames":{"pattern":"^(?<n>.)\\k<n>"}}),
        NamedBackreferenceKeysValidator::is_valid(&instance),
        NamedBackreferenceKeysValidator::validate(&instance),
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

    // A `$dynamicRef` back to its own resource beside `unevaluatedProperties`.
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

    // A `$dynamicRef` back to its own resource beside `unevaluatedItems`.
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

// A missing-required error precedes the additionalProperties error.
#[jsonschema::validator(
    schema = r#"{"properties":{"m":{"type":"integer"}},"required":["m"],"additionalProperties":false}"#
)]
struct AdditionalPropsFalseRequiredParityValidator;

#[test_case(serde_json::json!({"m":"x","z":1}) ; "value_error_first")]
#[test_case(serde_json::json!({"z":1}) ; "missing_required_before_additional")]
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

// `required` is validated before property values, so a missing required field is reported ahead
// of a covered-value error.
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

#[jsonschema::validator(
    schema = r#"{"properties":{"a":{"type":"integer"},"b":{"properties":{"c":{"type":"string"}}}},"required":["a","z"],"additionalProperties":{"type":"boolean"}}"#
)]
struct RequiredWithAdditionalSchemaParityValidator;

#[test_case(serde_json::json!({"a":1,"z":true}) ; "valid")]
#[test_case(serde_json::json!({"a":1,"z":0}) ; "required_key_checked_by_additional_properties")]
#[test_case(serde_json::json!({"a":"x","z":true}) ; "bad_value")]
#[test_case(serde_json::json!({"a":1,"b":{"c":1},"z":true}) ; "nested_bad_value")]
#[test_case(serde_json::json!({"b":{"c":1},"a":"x","z":true}) ; "first_bad_value_in_instance_order")]
#[test_case(serde_json::json!({"a":1,"y":1,"z":true}) ; "bad_additional_value")]
#[test_case(serde_json::json!({"a":"x","y":1}) ; "missing_required_not_in_properties")]
#[test_case(serde_json::json!({"z":0,"y":1}) ; "missing_required_in_properties")]
fn test_required_with_additional_schema_validate_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"properties":{"a":{"type":"integer"},"b":{"properties":{"c":{"type":"string"}}}},"required":["a","z"],"additionalProperties":{"type":"boolean"}});
    assert_validate_parity_for(
        &schema,
        RequiredWithAdditionalSchemaParityValidator::is_valid(&instance),
        RequiredWithAdditionalSchemaParityValidator::validate(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"],"additionalProperties":false}"#
)]
struct RequiredWithAdditionalFalseParityValidator;

#[test_case(serde_json::json!({"a":1,"b":2}) ; "valid")]
#[test_case(serde_json::json!({"a":1,"b":2,"x":0}) ; "unexpected_key")]
#[test_case(serde_json::json!({"a":1,"x":0}) ; "missing_required_and_unexpected_key")]
#[test_case(serde_json::json!({"x":0,"a":"y","b":2}) ; "unexpected_key_before_bad_value")]
fn test_required_with_additional_false_validate_parity(instance: serde_json::Value) {
    let schema = serde_json::json!({"properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"],"additionalProperties":false});
    assert_validate_parity_for(
        &schema,
        RequiredWithAdditionalFalseParityValidator::is_valid(&instance),
        RequiredWithAdditionalFalseParityValidator::validate(&instance),
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

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root.json","$ref":"tree.json","$defs":{"node":{"$dynamicAnchor":"node","$ref":"#/$defs/leaf"},"leaf":{"properties":{"v":{"type":"integer"}}}}}"##,
    resources = {
        "https://example.com/tree.json" => { schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/tree.json","$dynamicAnchor":"node","type":"object","properties":{"child":{"$dynamicRef":"#node"}},"$defs":{"leaf":{"type":"string"}}}"## },
    }
)]
struct CrossDocumentDynamicRefValidator;

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

#[jsonschema::validator(
    schema = r##"{"$id":"https://ex/root","$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"properties":{"value":{"type":"integer"}},"allOf":[{"$ref":"https://ex/sub"}],"$defs":{"sub":{"$id":"https://ex/sub","$recursiveAnchor":true,"properties":{"child":{"$ref":"#/$defs/wrapper"}},"$defs":{"wrapper":{"$recursiveRef":"#"}}}}}"##,
    draft = referencing::Draft::Draft201909
)]
struct RecursiveRefCrossResourceValidator;

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

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/format-assertion","type":"string","format":"date-time"}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/format-assertion" => { schema = r#"{"$id":"json-schema:///meta/format-assertion","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/format-assertion":true}}"# },
    }
)]
struct FormatAssertionVocabularyValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/optional-format-assertion","type":"string","format":"date-time"}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/optional-format-assertion" => { schema = r#"{"$id":"json-schema:///meta/optional-format-assertion","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/format-assertion":false}}"# },
    }
)]
struct OptionalFormatAssertionVocabularyValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/draft2019-format","type":"string","format":"date-time"}"#,
    draft = referencing::Draft::Draft201909,
    resources = {
        "json-schema:///meta/draft2019-format" => { schema = r#"{"$id":"json-schema:///meta/draft2019-format","$schema":"https://json-schema.org/draft/2019-09/schema","$vocabulary":{"https://json-schema.org/draft/2019-09/vocab/core":true,"https://json-schema.org/draft/2019-09/vocab/validation":true,"https://json-schema.org/draft/2019-09/vocab/format":true}}"# },
    }
)]
struct Draft201909FormatVocabularyValidator;

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/custom-vocab","type":"string"}"#,
    draft = referencing::Draft::Draft202012,
    vocabularies = ["https://example.com/vocab/made-up"],
    resources = {
        "json-schema:///meta/custom-vocab" => { schema = r#"{"$id":"json-schema:///meta/custom-vocab","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://example.com/vocab/made-up":true}}"# },
    }
)]
struct DeclaredVocabularyValidator;

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

#[test_case(serde_json::json!({}) ; "empty_object")]
#[test_case(serde_json::json!({"a": 1}) ; "one_property")]
#[test_case(serde_json::json!({"a": 1, "c": 2}) ; "two_properties")]
fn test_additional_properties_false_errors_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({"type":"object","additionalProperties":false});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        AdditionalPropertiesFalseValidator::is_valid(&instance),
        AdditionalPropertiesFalseValidator::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        AdditionalPropertiesFalseValidator::iter_errors(&instance),
        &runtime,
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

#[test_case(serde_json::json!({"child": {"value": "bad"}}), false ; "cross_resource_recurses_to_root_reject")]
#[test_case(serde_json::json!({"child": {"value": 5}}), true ; "cross_resource_recurses_to_root_accept")]
fn test_recursive_ref_cross_resource_match_runtime(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({"$id":"https://ex/root","$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveAnchor":true,"properties":{"value":{"type":"integer"}},"allOf":[{"$ref":"https://ex/sub"}],"$defs":{"sub":{"$id":"https://ex/sub","$recursiveAnchor":true,"properties":{"child":{"$ref":"#/$defs/wrapper"}},"$defs":{"wrapper":{"$recursiveRef":"#"}}}}});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(
        runtime.is_valid(&instance),
        expected,
        "runtime oracle for {instance}"
    );
    assert_eq!(
        RecursiveRefCrossResourceValidator::is_valid(&instance),
        expected,
        "codegen/runtime mismatch for {instance}"
    );
}

#[test_case(serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "anyOf": [{"type": "string"}, {"type": 1}]}), false ; "nested_illegal_type_rejected")]
#[test_case(serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "anyOf": [{"type": "string"}, {"type": "integer"}]}), true ; "nested_legal_type_accepted")]
fn test_generated_2019_09_metaschema(schema: serde_json::Value, expected: bool) {
    assert_eq!(jsonschema::meta::is_valid(&schema), expected);
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
#[test_case(r#"{"type":"number","exclusiveMaximum":0.1}"#, "1e400"; "exclusive_maximum_bigfrac_vs_past_range_integer")]
#[test_case(r#"{"type":"number","exclusiveMinimum":0.1}"#, "-1e400"; "exclusive_minimum_bigfrac_vs_past_range_integer")]
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
#[jsonschema::validator(schema = r#"{"const":0}"#)]
struct ArbitraryPrecisionConstZeroValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"maximum":0}"#)]
struct ArbitraryPrecisionMaximumZeroValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"multipleOf":2}"#)]
struct ArbitraryPrecisionMultipleOfTwoValidator;

#[cfg(feature = "arbitrary-precision")]
#[jsonschema::validator(schema = r#"{"multipleOf":1e-2000000}"#)]
struct ArbitraryPrecisionTinyMultipleOfValidator;

// Numbers with more digits than exact arithmetic materialises.
#[cfg(feature = "arbitrary-precision")]
#[test_case(ArbitraryPrecisionIntegerTypeValidator::is_valid, "1e-2000000", false)]
#[test_case(ArbitraryPrecisionIntegerTypeValidator::is_valid, "1e2000000", true)]
#[test_case(ArbitraryPrecisionConstZeroValidator::is_valid, "1e-2000000", false)]
#[test_case(ArbitraryPrecisionMaximumZeroValidator::is_valid, "1e-2000000", false)]
#[test_case(ArbitraryPrecisionMaximumZeroValidator::is_valid, "-1e-2000000", true)]
#[test_case(ArbitraryPrecisionMultipleOfTwoValidator::is_valid, "1e2000000", true)]
#[test_case(ArbitraryPrecisionTinyMultipleOfValidator::is_valid, "3", true)]
#[test_case(
    ArbitraryPrecisionTinyMultipleOfValidator::is_valid,
    "1e-2000001",
    false
)]
#[test_case(
    ArbitraryPrecisionTinyExclusiveMinimumValidator::is_valid,
    "2e-2000000",
    true
)]
#[test_case(
    ArbitraryPrecisionTinyExclusiveMinimumValidator::is_valid,
    "0.5e-2000000",
    false
)]
fn test_arbitrary_precision_codegen_past_digit_budget(
    is_valid: fn(&serde_json::Value) -> bool,
    instance: &str,
    expected: bool,
) {
    let instance: serde_json::Value = serde_json::from_str(instance).expect("valid instance json");
    assert_eq!(is_valid(&instance), expected);
}

// A divisor beyond `f64`.
#[cfg(feature = "arbitrary-precision")]
#[test_case(ArbitraryPrecisionMultipleOfExtremeValidator::evaluate, r#"{"type":"number","multipleOf":1e400}"#, "1" ; "extreme_vs_one")]
#[test_case(ArbitraryPrecisionMultipleOfExtremeValidator::evaluate, r#"{"type":"number","multipleOf":1e400}"#, "2e400" ; "extreme_vs_multiple")]
#[test_case(ArbitraryPrecisionHugeMultipleOfValidator::evaluate, r#"{"type":"number","multipleOf":1e2000000}"#, "1" ; "huge_vs_one")]
#[test_case(ArbitraryPrecisionHugeMultipleOfValidator::evaluate, r#"{"type":"number","multipleOf":1e2000000}"#, "0" ; "huge_vs_zero")]
fn test_multiple_of_beyond_f64_evaluate_matches_runtime(
    evaluate: EvaluateFn,
    schema_json: &str,
    instance_json: &str,
) {
    let schema: serde_json::Value = serde_json::from_str(schema_json).expect("valid schema json");
    let instance: serde_json::Value =
        serde_json::from_str(instance_json).expect("valid instance json");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_eq!(
        evaluation_outputs(&evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
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

// The branch's second hop resolves inside `other`, whose `a` accepts another `kind`.
#[jsonschema::validator(
    schema = r##"{"$defs": {"a": {"required": ["kind"], "properties": {"kind": {"const": "x"}}}, "other": {"$id": "http://example.com/other", "$defs": {"hop": {"$ref": "#/$defs/a"}, "a": {"required": ["kind"], "properties": {"kind": {"const": "y"}}}}}}, "oneOf": [{"$ref": "http://example.com/other#/$defs/hop"}, {"required": ["kind"], "properties": {"kind": {"const": "z"}}}]}"##
)]
struct OneOfDiscriminatorThroughReferenceChain;

#[test_case(serde_json::json!({"kind": "x"}) ; "root_definition")]
#[test_case(serde_json::json!({"kind": "y"}) ; "resource_definition")]
#[test_case(serde_json::json!({"kind": "z"}) ; "inline_branch")]
fn test_one_of_discriminator_through_reference_chain_into_another_resource(
    instance: serde_json::Value,
) {
    let schema = serde_json::json!({"$defs": {"a": {"required": ["kind"], "properties": {"kind": {"const": "x"}}}, "other": {"$id": "http://example.com/other", "$defs": {"hop": {"$ref": "#/$defs/a"}, "a": {"required": ["kind"], "properties": {"kind": {"const": "y"}}}}}}, "oneOf": [{"$ref": "http://example.com/other#/$defs/hop"}, {"required": ["kind"], "properties": {"kind": {"const": "z"}}}]});
    assert_is_valid_parity(
        &schema,
        OneOfDiscriminatorThroughReferenceChain::is_valid(&instance),
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

// The dynamic target's `$ref` resolves against the document holding the target
#[test_case(serde_json::json!({"child": {"v": 1}}), None ; "target_relative_ref_accepts_integer")]
#[test_case(
    serde_json::json!({"child": {"v": "x"}}),
    Some(("\"x\" is not of type \"integer\"", "/$defs/leaf/properties/v/type"))
    ; "target_relative_ref_rejects_string"
)]
#[test_case(serde_json::json!({"child": "str"}), None ; "target_permits_non_object")]
fn test_cross_document_dynamic_ref_uses_target_base(
    instance: serde_json::Value,
    expected_error: Option<(&str, &str)>,
) {
    assert_eq!(
        CrossDocumentDynamicRefValidator::is_valid(&instance),
        expected_error.is_none()
    );
    let result = CrossDocumentDynamicRefValidator::validate(&instance);
    match expected_error {
        Some((message, schema_path)) => {
            let error = result.expect_err("Should fail");
            assert_eq!(error.to_string(), message);
            assert_eq!(error.schema_path().as_str(), schema_path);
        }
        None => assert!(result.is_ok()),
    }
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

const REF_CYCLE_IN_BRANCHES: &str = r##"{"definitions": {"node": {"oneOf": [{"required": ["a"], "properties": {"a": {"const": 1}, "c": {"$ref": "#/definitions/node"}, "d": {"$ref": "#/definitions/node"}}}, {"properties": {"b": {"const": 1}, "c": {"$ref": "#/definitions/node"}, "d": {"$ref": "#/definitions/node"}}}, {"type": "string"}]}}, "$ref": "#/definitions/node"}"##;

#[jsonschema::validator(
    schema = r##"{"definitions": {"node": {"oneOf": [{"required": ["a"], "properties": {"a": {"const": 1}, "c": {"$ref": "#/definitions/node"}, "d": {"$ref": "#/definitions/node"}}}, {"properties": {"b": {"const": 1}, "c": {"$ref": "#/definitions/node"}, "d": {"$ref": "#/definitions/node"}}}, {"type": "string"}]}}, "$ref": "#/definitions/node"}"##,
    draft = Draft7
)]
struct RefCycleInBranchesValidator;

// Two `$ref` cycles that each reach every instance twice, one accepting what the other rejects.
#[jsonschema::validator(
    schema = r##"{"definitions": {"any": {"properties": {"c": {"$ref": "#/definitions/any"}}, "allOf": [{"properties": {"c": {"$ref": "#/definitions/any"}}}]}, "required": {"required": ["c"], "properties": {"c": {"$ref": "#/definitions/required"}}, "allOf": [{"properties": {"c": {"$ref": "#/definitions/required"}}}]}}, "allOf": [{"$ref": "#/definitions/any"}, {"$ref": "#/definitions/required"}]}"##,
    draft = Draft7
)]
struct TwoRefCyclesValidator;

// Each level of `levels` wraps the one below as `c`: `a` sends it through the first two `oneOf`
// branches of `REF_CYCLE_IN_BRANCHES`, `b` through one.
fn nested(levels: &str, leaf: serde_json::Value) -> serde_json::Value {
    levels.chars().rev().fold(leaf, |child, level| match level {
        'a' => serde_json::json!({"a": 1, "c": child}),
        _ => serde_json::json!({"b": 1, "c": child}),
    })
}

#[test_case(nested(&"b".repeat(20), serde_json::json!({})), true ; "single_branch_chain")]
#[test_case(nested(&"a".repeat(20), serde_json::json!({})), false ; "both_branches_chain")]
#[test_case(nested(&"b".repeat(20), serde_json::json!({"b": 2})), false ; "single_branch_chain_invalid_leaf")]
#[test_case(nested(&format!("{}{}", "b".repeat(4), "a".repeat(16)), serde_json::json!({})), false ; "both_branches_below_single")]
#[test_case(serde_json::json!({"b": 1, "c": nested(&"b".repeat(18), serde_json::json!({})), "d": nested(&"a".repeat(6), serde_json::json!({}))}), false ; "valid_sibling_before_invalid")]
#[test_case(serde_json::json!({"b": 1, "c": nested(&"a".repeat(6), serde_json::json!({})), "d": nested(&"b".repeat(18), serde_json::json!({}))}), false ; "invalid_sibling_before_valid")]
#[test_case(serde_json::json!({"b": 1, "c": nested(&"b".repeat(18), serde_json::json!({})), "d": nested(&"b".repeat(18), serde_json::json!({}))}), true ; "valid_siblings")]
fn test_ref_cycle_in_branches_matches_runtime(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::from_str(REF_CYCLE_IN_BRANCHES).expect("valid JSON");
    let runtime = jsonschema::draft7::new(&schema).expect("valid schema");
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(RefCycleInBranchesValidator::is_valid(&instance), expected);
}

#[test]
fn test_two_ref_cycles_on_the_same_instances() {
    let instance = (0..20).fold(
        serde_json::json!({}),
        |child, _| serde_json::json!({"c": child}),
    );
    assert!(!TwoRefCyclesValidator::is_valid(&instance));
}

#[test]
fn test_ref_cycle_results_do_not_outlive_their_instance() {
    for round in 0..8 {
        let leaf = if round % 2 == 0 {
            serde_json::json!({})
        } else {
            serde_json::json!({"b": 2})
        };
        let instance = nested(&"b".repeat(20), leaf);
        assert_eq!(
            RefCycleInBranchesValidator::is_valid(&instance),
            round % 2 == 0
        );
    }
}

const NESTED_THEN_SAME_INSTANCE_CYCLE: &str = r##"{"$ref":"#/$defs/cycle","$defs":{"cycle":{"minProperties":1,"allOf":[{"$ref":"#/$defs/cycle"}],"additionalProperties":{"$ref":"#/$defs/cycle"}}}}"##;
const NESTED_THEN_SAME_INSTANCE_NOT_CYCLE: &str = r##"{"$ref":"#/$defs/c","$defs":{"c":{"not":{"$ref":"#/$defs/c"},"additionalProperties":{"$ref":"#/$defs/c"}}}}"##;
const NESTED_THEN_SAME_INSTANCE_CYCLE_THROUGH_TARGET: &str = r##"{"$ref":"#/$defs/t","$defs":{"t":{"additionalProperties":{"$ref":"#/$defs/w"},"allOf":[{"$ref":"#/$defs/w"}]},"w":{"minProperties":1,"allOf":[{"$ref":"#/$defs/t"}]}}}"##;
const NESTED_THEN_SAME_INSTANCE_UNANCHORED_RECURSIVE_REF: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","minProperties":1,"allOf":[{"$recursiveRef":"#"}],"additionalProperties":{"$recursiveRef":"#"}}"##;
const NESTED_THEN_SAME_INSTANCE_UNANCHORED_DYNAMIC_REF: &str = r##"{"$ref":"#/$defs/cycle","$defs":{"cycle":{"minProperties":1,"allOf":[{"$dynamicRef":"#/$defs/cycle"}],"additionalProperties":{"$dynamicRef":"#/$defs/cycle"}}}}"##;
const NESTED_THEN_SAME_INSTANCE_TWO_PATHS: &str = r##"{"$ref":"#/$defs/t","$defs":{"t":{"allOf":[{"$ref":"#/$defs/t"}],"properties":{"c":{"$ref":"#/$defs/t"}},"patternProperties":{"^c":{"$ref":"#/$defs/t"}}}}}"##;

// Each cycle reaches a nested instance first and comes back to that instance through `allOf` or
// `not`.
#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/cycle","$defs":{"cycle":{"minProperties":1,"allOf":[{"$ref":"#/$defs/cycle"}],"additionalProperties":{"$ref":"#/$defs/cycle"}}}}"##,
    draft = Draft202012
)]
struct NestedThenSameInstanceCycle;

#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/c","$defs":{"c":{"not":{"$ref":"#/$defs/c"},"additionalProperties":{"$ref":"#/$defs/c"}}}}"##,
    draft = Draft202012
)]
struct NestedThenSameInstanceNotCycle;

#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/t","$defs":{"t":{"additionalProperties":{"$ref":"#/$defs/w"},"allOf":[{"$ref":"#/$defs/w"}]},"w":{"minProperties":1,"allOf":[{"$ref":"#/$defs/t"}]}}}"##,
    draft = Draft202012
)]
struct NestedThenSameInstanceCycleThroughTarget;

// Without an anchor, `$recursiveRef` and `$dynamicRef` apply the schema they name.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","minProperties":1,"allOf":[{"$recursiveRef":"#"}],"additionalProperties":{"$recursiveRef":"#"}}"##,
    draft = Draft201909
)]
struct NestedThenSameInstanceUnanchoredRecursiveRef;

#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/cycle","$defs":{"cycle":{"minProperties":1,"allOf":[{"$dynamicRef":"#/$defs/cycle"}],"additionalProperties":{"$dynamicRef":"#/$defs/cycle"}}}}"##,
    draft = Draft202012
)]
struct NestedThenSameInstanceUnanchoredDynamicRef;

// Two keywords reach each nested instance.
#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/t","$defs":{"t":{"allOf":[{"$ref":"#/$defs/t"}],"properties":{"c":{"$ref":"#/$defs/t"}},"patternProperties":{"^c":{"$ref":"#/$defs/t"}}}}}"##,
    draft = Draft202012
)]
struct NestedThenSameInstanceTwoPaths;

// Each level holds the next one as `c`.
fn chain_of_c(depth: usize) -> serde_json::Value {
    (0..depth).fold(
        serde_json::json!({}),
        |child, _| serde_json::json!({"c": child}),
    )
}

#[test_case(NestedThenSameInstanceCycle::is_valid, NestedThenSameInstanceCycle::validate, NestedThenSameInstanceCycle::iter_errors, NESTED_THEN_SAME_INSTANCE_CYCLE, serde_json::json!({}) ; "cycle_root")]
#[test_case(NestedThenSameInstanceCycle::is_valid, NestedThenSameInstanceCycle::validate, NestedThenSameInstanceCycle::iter_errors, NESTED_THEN_SAME_INSTANCE_CYCLE, serde_json::json!({"a": {}}) ; "cycle_one_level")]
#[test_case(NestedThenSameInstanceCycle::is_valid, NestedThenSameInstanceCycle::validate, NestedThenSameInstanceCycle::iter_errors, NESTED_THEN_SAME_INSTANCE_CYCLE, serde_json::json!({"a": {"b": {"c": {}}}, "d": {}}) ; "cycle_three_levels")]
#[test_case(NestedThenSameInstanceNotCycle::is_valid, NestedThenSameInstanceNotCycle::validate, NestedThenSameInstanceNotCycle::iter_errors, NESTED_THEN_SAME_INSTANCE_NOT_CYCLE, serde_json::json!({"a": {}}) ; "not_cycle_one_level")]
#[test_case(NestedThenSameInstanceNotCycle::is_valid, NestedThenSameInstanceNotCycle::validate, NestedThenSameInstanceNotCycle::iter_errors, NESTED_THEN_SAME_INSTANCE_NOT_CYCLE, serde_json::json!({"a": {"b": {}}}) ; "not_cycle_two_levels")]
#[test_case(NestedThenSameInstanceCycleThroughTarget::is_valid, NestedThenSameInstanceCycleThroughTarget::validate, NestedThenSameInstanceCycleThroughTarget::iter_errors, NESTED_THEN_SAME_INSTANCE_CYCLE_THROUGH_TARGET, serde_json::json!({}) ; "through_target_root")]
#[test_case(NestedThenSameInstanceCycleThroughTarget::is_valid, NestedThenSameInstanceCycleThroughTarget::validate, NestedThenSameInstanceCycleThroughTarget::iter_errors, NESTED_THEN_SAME_INSTANCE_CYCLE_THROUGH_TARGET, serde_json::json!({"a": {"b": {}}}) ; "through_target_two_levels")]
#[test_case(NestedThenSameInstanceUnanchoredRecursiveRef::is_valid, NestedThenSameInstanceUnanchoredRecursiveRef::validate, NestedThenSameInstanceUnanchoredRecursiveRef::iter_errors, NESTED_THEN_SAME_INSTANCE_UNANCHORED_RECURSIVE_REF, serde_json::json!({"a": {"b": {"c": {}}}}) ; "unanchored_recursive_ref")]
#[test_case(NestedThenSameInstanceUnanchoredDynamicRef::is_valid, NestedThenSameInstanceUnanchoredDynamicRef::validate, NestedThenSameInstanceUnanchoredDynamicRef::iter_errors, NESTED_THEN_SAME_INSTANCE_UNANCHORED_DYNAMIC_REF, serde_json::json!({"a": {"b": {"c": {}}}}) ; "unanchored_dynamic_ref")]
#[test_case(NestedThenSameInstanceTwoPaths::is_valid, NestedThenSameInstanceTwoPaths::validate, NestedThenSameInstanceTwoPaths::iter_errors, NESTED_THEN_SAME_INSTANCE_TWO_PATHS, chain_of_c(4) ; "two_paths")]
fn test_ref_cycle_into_nested_then_same_instance_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    schema: &str,
    instance: serde_json::Value,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        is_valid(&instance),
        validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
}

// Without the results kept for the run, the work doubles at every level.
#[test]
fn test_ref_cycle_into_nested_then_same_instance_keeps_results() {
    let instance = chain_of_c(64);
    assert!(NestedThenSameInstanceTwoPaths::is_valid(&instance));
    assert!(NestedThenSameInstanceTwoPaths::validate(&instance).is_ok());
    assert_eq!(
        NestedThenSameInstanceTwoPaths::iter_errors(&instance).count(),
        0
    );
}

// Schemas naming one `$ref` target in different ways, and different targets in similar ways.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"cycle","$defs":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#"}}}}"##,
    draft = Draft201909
)]
struct RefCycleResourceAndRootFragment;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"cycle#","$defs":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#"}}}}"##,
    draft = Draft202012
)]
struct RefCycleResourceWithEmptyFragment;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"https://example.com/cycle","$defs":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"cycle"}],"additionalProperties":{"$ref":"#"}}}}"##,
    draft = Draft202012
)]
struct RefCycleAbsoluteAndRelativeUri;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"cycle","$defs":{"cycle":{"$id":"cycle","$anchor":"node","minProperties":1,"allOf":[{"$ref":"#node"}],"additionalProperties":{"$ref":"#"}}}}"##,
    draft = Draft202012
)]
struct RefCycleAnchorAndRootFragment;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"cycle#node","$defs":{"cycle":{"$id":"cycle","$anchor":"node","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#node"}}}}"##,
    draft = Draft202012
)]
struct RefCycleAnchorFromAnotherResource;
#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/cycle","$defs":{"cycle":{"$anchor":"node","minProperties":1,"allOf":[{"$ref":"#node"}],"additionalProperties":{"$ref":"#/$defs/cycle"}}}}"##,
    draft = Draft202012
)]
struct RefCycleAnchorAndPointer;
#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/cycle","$defs":{"cycle":{"minProperties":1,"allOf":[{"$ref":"#/%24defs/cycle"}],"additionalProperties":{"$ref":"#/$defs/%63ycle"}}}}"##,
    draft = Draft202012
)]
struct RefCyclePercentEncodedPointer;
#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/a%23%25b","$defs":{"a#%b":{"minProperties":1,"allOf":[{"$ref":"#/$defs/a%23%25%62"}],"additionalProperties":{"$ref":"#/$defs/a%23%25b"}}}}"##,
    draft = Draft202012
)]
struct RefCyclePercentEncodedNumberSignAndPercent;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/cycle","$defs":{"leaf":{"$id":"leaf","minProperties":1},"cycle":{"$id":"cycle","allOf":[{"$ref":"leaf"},{"$ref":"root#/$defs/cycle"}],"additionalProperties":{"$ref":"https://example.com/root#/$defs/cycle"}}}}"##,
    draft = Draft202012
)]
struct RefCyclePointerThroughResource;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/cycle","$defs":{"leaf":{"$id":"leaf","minProperties":1},"cycle":{"$id":"cycle","allOf":[{"$ref":"leaf"},{"$ref":"#"}],"additionalProperties":{"$ref":"root#/$defs/cycle"}}}}"##,
    draft = Draft202012
)]
struct RefCyclePointerThroughResourceAndRootFragment;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"https://example.com/root"}}"##,
    draft = Draft202012
)]
struct RefCycleRootAndItsUri;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","":{"maxProperties":0},"minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#/"}}"##,
    draft = Draft202012
)]
struct RefCycleRootAndEmptyNameMember;
#[jsonschema::validator(
    schema = r#"{"$id":"https://example.com/root","properties":{"a":{"$ref":"root#/$defs/cycle"},"b":{"$ref":"cycle#/$defs/cycle"}},"$defs":{"cycle":{"$id":"cycle","$defs":{"cycle":{"maxProperties":0}}}}}"#,
    draft = Draft202012
)]
struct RefPointerThroughResourceAndPointerInIt;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","allOf":[{"$ref":"cycle"}],"definitions":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#"}}}}"##,
    draft = Draft7
)]
struct RefCycleResourceDraft7;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","allOf":[{"$ref":"#node"}],"definitions":{"cycle":{"$id":"#node","minProperties":1,"allOf":[{"$ref":"#/definitions/cycle"}],"additionalProperties":{"$ref":"#node"}}}}"##,
    draft = Draft7
)]
struct RefCyclePlainNameIdDraft7;
#[jsonschema::validator(
    schema = r##"{"id":"https://example.com/root","allOf":[{"$ref":"cycle"}],"definitions":{"cycle":{"id":"cycle","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#"}}}}"##,
    draft = Draft4
)]
struct RefCycleResourceDraft4;
#[jsonschema::validator(
    schema = r##"{"id":"https://example.com/root","allOf":[{"$ref":"#node"}],"definitions":{"cycle":{"id":"#node","minProperties":1,"allOf":[{"$ref":"https://example.com/root#/definitions/cycle"}],"additionalProperties":{"$ref":"#node"}}}}"##,
    draft = Draft4
)]
struct RefCyclePlainNameIdDraft4;

// Every way of naming a target within one document gives one function, as it gives one node in the
// runtime.
#[test_case(RefCycleResourceAndRootFragment::is_valid, RefCycleResourceAndRootFragment::validate, RefCycleResourceAndRootFragment::iter_errors, RefCycleResourceAndRootFragment::evaluate, r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"cycle","$defs":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#"}}}}"##, Draft::Draft201909 ; "resource_and_root_fragment_draft2019_09")]
#[test_case(RefCycleResourceWithEmptyFragment::is_valid, RefCycleResourceWithEmptyFragment::validate, RefCycleResourceWithEmptyFragment::iter_errors, RefCycleResourceWithEmptyFragment::evaluate, r##"{"$id":"https://example.com/root","$ref":"cycle#","$defs":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#"}}}}"##, Draft::Draft202012 ; "resource_with_empty_fragment")]
#[test_case(RefCycleAbsoluteAndRelativeUri::is_valid, RefCycleAbsoluteAndRelativeUri::validate, RefCycleAbsoluteAndRelativeUri::iter_errors, RefCycleAbsoluteAndRelativeUri::evaluate, r##"{"$id":"https://example.com/root","$ref":"https://example.com/cycle","$defs":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"cycle"}],"additionalProperties":{"$ref":"#"}}}}"##, Draft::Draft202012 ; "absolute_and_relative_uri")]
#[test_case(RefCycleAnchorAndRootFragment::is_valid, RefCycleAnchorAndRootFragment::validate, RefCycleAnchorAndRootFragment::iter_errors, RefCycleAnchorAndRootFragment::evaluate, r##"{"$id":"https://example.com/root","$ref":"cycle","$defs":{"cycle":{"$id":"cycle","$anchor":"node","minProperties":1,"allOf":[{"$ref":"#node"}],"additionalProperties":{"$ref":"#"}}}}"##, Draft::Draft202012 ; "anchor_and_root_fragment")]
#[test_case(RefCycleAnchorFromAnotherResource::is_valid, RefCycleAnchorFromAnotherResource::validate, RefCycleAnchorFromAnotherResource::iter_errors, RefCycleAnchorFromAnotherResource::evaluate, r##"{"$id":"https://example.com/root","$ref":"cycle#node","$defs":{"cycle":{"$id":"cycle","$anchor":"node","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#node"}}}}"##, Draft::Draft202012 ; "anchor_from_another_resource")]
#[test_case(RefCycleAnchorAndPointer::is_valid, RefCycleAnchorAndPointer::validate, RefCycleAnchorAndPointer::iter_errors, RefCycleAnchorAndPointer::evaluate, r##"{"$ref":"#/$defs/cycle","$defs":{"cycle":{"$anchor":"node","minProperties":1,"allOf":[{"$ref":"#node"}],"additionalProperties":{"$ref":"#/$defs/cycle"}}}}"##, Draft::Draft202012 ; "anchor_and_pointer")]
#[test_case(RefCyclePercentEncodedPointer::is_valid, RefCyclePercentEncodedPointer::validate, RefCyclePercentEncodedPointer::iter_errors, RefCyclePercentEncodedPointer::evaluate, r##"{"$ref":"#/$defs/cycle","$defs":{"cycle":{"minProperties":1,"allOf":[{"$ref":"#/%24defs/cycle"}],"additionalProperties":{"$ref":"#/$defs/%63ycle"}}}}"##, Draft::Draft202012 ; "percent_encoded_pointer")]
#[test_case(RefCyclePercentEncodedNumberSignAndPercent::is_valid, RefCyclePercentEncodedNumberSignAndPercent::validate, RefCyclePercentEncodedNumberSignAndPercent::iter_errors, RefCyclePercentEncodedNumberSignAndPercent::evaluate, r##"{"$ref":"#/$defs/a%23%25b","$defs":{"a#%b":{"minProperties":1,"allOf":[{"$ref":"#/$defs/a%23%25%62"}],"additionalProperties":{"$ref":"#/$defs/a%23%25b"}}}}"##, Draft::Draft202012 ; "percent_encoded_number_sign_and_percent")]
#[test_case(RefCyclePointerThroughResource::is_valid, RefCyclePointerThroughResource::validate, RefCyclePointerThroughResource::iter_errors, RefCyclePointerThroughResource::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/cycle","$defs":{"leaf":{"$id":"leaf","minProperties":1},"cycle":{"$id":"cycle","allOf":[{"$ref":"leaf"},{"$ref":"root#/$defs/cycle"}],"additionalProperties":{"$ref":"https://example.com/root#/$defs/cycle"}}}}"##, Draft::Draft202012 ; "pointer_through_resource")]
#[test_case(RefCyclePointerThroughResourceAndRootFragment::is_valid, RefCyclePointerThroughResourceAndRootFragment::validate, RefCyclePointerThroughResourceAndRootFragment::iter_errors, RefCyclePointerThroughResourceAndRootFragment::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/cycle","$defs":{"leaf":{"$id":"leaf","minProperties":1},"cycle":{"$id":"cycle","allOf":[{"$ref":"leaf"},{"$ref":"#"}],"additionalProperties":{"$ref":"root#/$defs/cycle"}}}}"##, Draft::Draft202012 ; "pointer_through_resource_and_root_fragment")]
#[test_case(RefCycleRootAndItsUri::is_valid, RefCycleRootAndItsUri::validate, RefCycleRootAndItsUri::iter_errors, RefCycleRootAndItsUri::evaluate, r##"{"$id":"https://example.com/root","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"https://example.com/root"}}"##, Draft::Draft202012 ; "root_and_its_uri")]
#[test_case(RefCycleRootAndEmptyNameMember::is_valid, RefCycleRootAndEmptyNameMember::validate, RefCycleRootAndEmptyNameMember::iter_errors, RefCycleRootAndEmptyNameMember::evaluate, r##"{"$id":"https://example.com/root","":{"maxProperties":0},"minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#/"}}"##, Draft::Draft202012 ; "root_and_empty_name_member")]
#[test_case(RefPointerThroughResourceAndPointerInIt::is_valid, RefPointerThroughResourceAndPointerInIt::validate, RefPointerThroughResourceAndPointerInIt::iter_errors, RefPointerThroughResourceAndPointerInIt::evaluate, r#"{"$id":"https://example.com/root","properties":{"a":{"$ref":"root#/$defs/cycle"},"b":{"$ref":"cycle#/$defs/cycle"}},"$defs":{"cycle":{"$id":"cycle","$defs":{"cycle":{"maxProperties":0}}}}}"#, Draft::Draft202012 ; "pointer_through_resource_and_pointer_in_it")]
#[test_case(RefCycleResourceDraft7::is_valid, RefCycleResourceDraft7::validate, RefCycleResourceDraft7::iter_errors, RefCycleResourceDraft7::evaluate, r##"{"$id":"https://example.com/root","allOf":[{"$ref":"cycle"}],"definitions":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#"}}}}"##, Draft::Draft7 ; "resource_and_root_fragment_draft7")]
#[test_case(RefCyclePlainNameIdDraft7::is_valid, RefCyclePlainNameIdDraft7::validate, RefCyclePlainNameIdDraft7::iter_errors, RefCyclePlainNameIdDraft7::evaluate, r##"{"$id":"https://example.com/root","allOf":[{"$ref":"#node"}],"definitions":{"cycle":{"$id":"#node","minProperties":1,"allOf":[{"$ref":"#/definitions/cycle"}],"additionalProperties":{"$ref":"#node"}}}}"##, Draft::Draft7 ; "plain_name_id_and_pointer_draft7")]
#[test_case(RefCycleResourceDraft4::is_valid, RefCycleResourceDraft4::validate, RefCycleResourceDraft4::iter_errors, RefCycleResourceDraft4::evaluate, r##"{"id":"https://example.com/root","allOf":[{"$ref":"cycle"}],"definitions":{"cycle":{"id":"cycle","minProperties":1,"allOf":[{"$ref":"#"}],"additionalProperties":{"$ref":"#"}}}}"##, Draft::Draft4 ; "resource_and_root_fragment_draft4")]
#[test_case(RefCyclePlainNameIdDraft4::is_valid, RefCyclePlainNameIdDraft4::validate, RefCyclePlainNameIdDraft4::iter_errors, RefCyclePlainNameIdDraft4::evaluate, r##"{"id":"https://example.com/root","allOf":[{"$ref":"#node"}],"definitions":{"cycle":{"id":"#node","minProperties":1,"allOf":[{"$ref":"https://example.com/root#/definitions/cycle"}],"additionalProperties":{"$ref":"#node"}}}}"##, Draft::Draft4 ; "plain_name_id_and_pointer_draft4")]
fn test_ref_target_named_in_different_ways_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
    draft: Draft,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("valid schema");
    for instance in [
        serde_json::json!({}),
        serde_json::json!({"a": {}}),
        serde_json::json!({"a": {"b": {}}}),
        serde_json::json!({"a": {"x": 1}}),
        serde_json::json!({"b": {}}),
        serde_json::json!({"b": {"x": 1}}),
        serde_json::json!(1),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
        );
    }
}

// Keywords inside a resource embedded in another one, reached in different ways.
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/cycle","$defs":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"root#/$defs/cycle"}],"additionalProperties":{"$ref":"https://example.com/root#/$defs/cycle"}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct PointerIntoResourceCycle;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct PointerBelowResourceRoot;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner/allOf/0","$defs":{"inner":{"$id":"inner","allOf":[{"minProperties":1,"properties":{"p":{"type":"string"}}}]}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct PointerToArrayItemInResource;
#[jsonschema::validator(
    schema = r#"{"$id":"https://example.com/root","$ref":"first","$defs":{"first":{"$id":"first","$ref":"root#/$defs/inner/$defs/leaf"},"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"#,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct RelativePointerFromAnotherResource;
#[jsonschema::validator(
    schema = r#"{"$id":"https://example.com/root","$ref":"first","$defs":{"first":{"$id":"first","$ref":"https://example.com/root#/$defs/inner/$defs/leaf"},"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"#,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct AbsolutePointerFromAnotherResource;
#[jsonschema::validator(
    schema = r#"{"$id":"https://example.com/root","$ref":"inner#leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"$anchor":"leaf","minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"#,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct AnchorInResource;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/outer/$defs/inner/$defs/leaf","$defs":{"outer":{"$id":"outer/","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct PointerThroughNestedResources;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/outer","$defs":{"outer":{"$id":"outer/","$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct PointerFromEmbeddedResource;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/a~1b%25/$defs/leaf","$defs":{"a/b%":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct EscapedPointerIntoResource;
#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"https://example.com/inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct PointerIntoResourceUnderDefaultBase;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$dynamicRef":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct DynamicRefPointerIntoResource;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}},"unevaluatedProperties":false}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct UnevaluatedInResourceReachedByPointer;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner","$defs":{"inner":{"$id":"inner","minProperties":1,"properties":{"p":{"$id":"deep","type":"string"}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct InlineResourceInTargetReachedByPointer;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner/$defs/never","$defs":{"inner":{"$id":"inner","$defs":{"never":false}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct FalseSchemaInResourceReachedByPointer;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner","$defs":{"inner":{"$id":"inner","minProperties":1,"properties":{"a":{"$ref":"#"},"p":{"type":"string"}}}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct ResourceRootFragmentCycle;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","minProperties":1,"properties":{"a":{"$ref":"#/$defs/inner"},"p":{"type":"string"}},"$defs":{"inner":{"$id":"inner","$ref":"root"}}}"##,
    draft = Draft202012,
    methods = { evaluate = true }
)]
struct RootReachedFromResource;
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##,
    draft = Draft201909,
    methods = { evaluate = true }
)]
struct PointerIntoResourceDraft201909;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","allOf":[{"$ref":"#/definitions/inner/definitions/leaf"}],"definitions":{"inner":{"$id":"inner","definitions":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##,
    draft = Draft7,
    methods = { evaluate = true }
)]
struct PointerIntoResourceDraft7;
#[jsonschema::validator(
    schema = r##"{"id":"https://example.com/root","allOf":[{"$ref":"#/definitions/inner/definitions/leaf"}],"definitions":{"inner":{"id":"inner","definitions":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##,
    draft = Draft4,
    methods = { evaluate = true }
)]
struct PointerIntoResourceDraft4;

// Their absolute locations count from the root of that resource, wherever a reference enters it.
#[test_case(PointerIntoResourceCycle::is_valid, PointerIntoResourceCycle::validate, PointerIntoResourceCycle::iter_errors, PointerIntoResourceCycle::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/cycle","$defs":{"cycle":{"$id":"cycle","minProperties":1,"allOf":[{"$ref":"root#/$defs/cycle"}],"additionalProperties":{"$ref":"https://example.com/root#/$defs/cycle"}}}}"##, Draft::Draft202012, "https://example.com/cycle#/minProperties" ; "pointer_into_resource_cycle")]
#[test_case(PointerBelowResourceRoot::is_valid, PointerBelowResourceRoot::validate, PointerBelowResourceRoot::iter_errors, PointerBelowResourceRoot::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##, Draft::Draft202012, "https://example.com/inner#/$defs/leaf/minProperties" ; "pointer_below_resource_root")]
#[test_case(PointerToArrayItemInResource::is_valid, PointerToArrayItemInResource::validate, PointerToArrayItemInResource::iter_errors, PointerToArrayItemInResource::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner/allOf/0","$defs":{"inner":{"$id":"inner","allOf":[{"minProperties":1,"properties":{"p":{"type":"string"}}}]}}}"##, Draft::Draft202012, "https://example.com/inner#/allOf/0/minProperties" ; "pointer_to_array_item_in_resource")]
#[test_case(RelativePointerFromAnotherResource::is_valid, RelativePointerFromAnotherResource::validate, RelativePointerFromAnotherResource::iter_errors, RelativePointerFromAnotherResource::evaluate, r#"{"$id":"https://example.com/root","$ref":"first","$defs":{"first":{"$id":"first","$ref":"root#/$defs/inner/$defs/leaf"},"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"#, Draft::Draft202012, "https://example.com/inner#/$defs/leaf/minProperties" ; "relative_pointer_from_another_resource")]
#[test_case(AbsolutePointerFromAnotherResource::is_valid, AbsolutePointerFromAnotherResource::validate, AbsolutePointerFromAnotherResource::iter_errors, AbsolutePointerFromAnotherResource::evaluate, r#"{"$id":"https://example.com/root","$ref":"first","$defs":{"first":{"$id":"first","$ref":"https://example.com/root#/$defs/inner/$defs/leaf"},"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"#, Draft::Draft202012, "https://example.com/inner#/$defs/leaf/minProperties" ; "absolute_pointer_from_another_resource")]
#[test_case(AnchorInResource::is_valid, AnchorInResource::validate, AnchorInResource::iter_errors, AnchorInResource::evaluate, r#"{"$id":"https://example.com/root","$ref":"inner#leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"$anchor":"leaf","minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"#, Draft::Draft202012, "https://example.com/inner#/$defs/leaf/minProperties" ; "anchor_in_resource")]
#[test_case(PointerThroughNestedResources::is_valid, PointerThroughNestedResources::validate, PointerThroughNestedResources::iter_errors, PointerThroughNestedResources::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/outer/$defs/inner/$defs/leaf","$defs":{"outer":{"$id":"outer/","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}}}"##, Draft::Draft202012, "https://example.com/outer/inner#/$defs/leaf/minProperties" ; "pointer_through_nested_resources")]
#[test_case(PointerFromEmbeddedResource::is_valid, PointerFromEmbeddedResource::validate, PointerFromEmbeddedResource::iter_errors, PointerFromEmbeddedResource::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/outer","$defs":{"outer":{"$id":"outer/","$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}}}"##, Draft::Draft202012, "https://example.com/outer/inner#/$defs/leaf/minProperties" ; "pointer_from_embedded_resource")]
#[test_case(EscapedPointerIntoResource::is_valid, EscapedPointerIntoResource::validate, EscapedPointerIntoResource::iter_errors, EscapedPointerIntoResource::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/a~1b%25/$defs/leaf","$defs":{"a/b%":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##, Draft::Draft202012, "https://example.com/inner#/$defs/leaf/minProperties" ; "escaped_pointer_into_resource")]
#[test_case(PointerIntoResourceUnderDefaultBase::is_valid, PointerIntoResourceUnderDefaultBase::validate, PointerIntoResourceUnderDefaultBase::iter_errors, PointerIntoResourceUnderDefaultBase::evaluate, r##"{"$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"https://example.com/inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##, Draft::Draft202012, "https://example.com/inner#/$defs/leaf/minProperties" ; "pointer_into_resource_under_default_base")]
#[test_case(DynamicRefPointerIntoResource::is_valid, DynamicRefPointerIntoResource::validate, DynamicRefPointerIntoResource::iter_errors, DynamicRefPointerIntoResource::evaluate, r##"{"$id":"https://example.com/root","$dynamicRef":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##, Draft::Draft202012, "https://example.com/inner#/$defs/leaf/minProperties" ; "dynamic_ref_pointer_into_resource")]
#[test_case(UnevaluatedInResourceReachedByPointer::is_valid, UnevaluatedInResourceReachedByPointer::validate, UnevaluatedInResourceReachedByPointer::iter_errors, UnevaluatedInResourceReachedByPointer::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}},"unevaluatedProperties":false}}}}}"##, Draft::Draft202012, "https://example.com/inner#/$defs/leaf/minProperties" ; "unevaluated_in_resource_reached_by_pointer")]
#[test_case(InlineResourceInTargetReachedByPointer::is_valid, InlineResourceInTargetReachedByPointer::validate, InlineResourceInTargetReachedByPointer::iter_errors, InlineResourceInTargetReachedByPointer::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner","$defs":{"inner":{"$id":"inner","minProperties":1,"properties":{"p":{"$id":"deep","type":"string"}}}}}"##, Draft::Draft202012, "https://example.com/inner#/minProperties" ; "inline_resource_in_target_reached_by_pointer")]
#[test_case(FalseSchemaInResourceReachedByPointer::is_valid, FalseSchemaInResourceReachedByPointer::validate, FalseSchemaInResourceReachedByPointer::iter_errors, FalseSchemaInResourceReachedByPointer::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner/$defs/never","$defs":{"inner":{"$id":"inner","$defs":{"never":false}}}}"##, Draft::Draft202012, "https://example.com/inner#/$defs/never" ; "false_schema_in_resource_reached_by_pointer")]
#[test_case(ResourceRootFragmentCycle::is_valid, ResourceRootFragmentCycle::validate, ResourceRootFragmentCycle::iter_errors, ResourceRootFragmentCycle::evaluate, r##"{"$id":"https://example.com/root","$ref":"#/$defs/inner","$defs":{"inner":{"$id":"inner","minProperties":1,"properties":{"a":{"$ref":"#"},"p":{"type":"string"}}}}}"##, Draft::Draft202012, "https://example.com/inner#/minProperties" ; "resource_root_fragment_cycle")]
#[test_case(RootReachedFromResource::is_valid, RootReachedFromResource::validate, RootReachedFromResource::iter_errors, RootReachedFromResource::evaluate, r##"{"$id":"https://example.com/root","minProperties":1,"properties":{"a":{"$ref":"#/$defs/inner"},"p":{"type":"string"}},"$defs":{"inner":{"$id":"inner","$ref":"root"}}}"##, Draft::Draft202012, "https://example.com/root#/minProperties" ; "root_reached_from_resource")]
#[test_case(PointerIntoResourceDraft201909::is_valid, PointerIntoResourceDraft201909::validate, PointerIntoResourceDraft201909::iter_errors, PointerIntoResourceDraft201909::evaluate, r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$ref":"#/$defs/inner/$defs/leaf","$defs":{"inner":{"$id":"inner","$defs":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##, Draft::Draft201909, "https://example.com/inner#/$defs/leaf/minProperties" ; "pointer_into_resource_draft2019_09")]
#[test_case(PointerIntoResourceDraft7::is_valid, PointerIntoResourceDraft7::validate, PointerIntoResourceDraft7::iter_errors, PointerIntoResourceDraft7::evaluate, r##"{"$id":"https://example.com/root","allOf":[{"$ref":"#/definitions/inner/definitions/leaf"}],"definitions":{"inner":{"$id":"inner","definitions":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##, Draft::Draft7, "https://example.com/inner#/definitions/leaf/minProperties" ; "pointer_into_resource_draft7")]
#[test_case(PointerIntoResourceDraft4::is_valid, PointerIntoResourceDraft4::validate, PointerIntoResourceDraft4::iter_errors, PointerIntoResourceDraft4::evaluate, r##"{"id":"https://example.com/root","allOf":[{"$ref":"#/definitions/inner/definitions/leaf"}],"definitions":{"inner":{"id":"inner","definitions":{"leaf":{"minProperties":1,"properties":{"p":{"type":"string"}}}}}}}"##, Draft::Draft4, "https://example.com/inner#/definitions/leaf/minProperties" ; "pointer_into_resource_draft4")]
fn test_keywords_in_embedded_resource_are_located_in_it(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
    draft: Draft,
    location: &str,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("valid schema");
    let rejected = serde_json::json!({});
    assert_eq!(
        validate(&rejected)
            .expect_err("`{}` is rejected")
            .absolute_keyword_location()
            .map(jsonschema::Uri::as_str),
        Some(location)
    );
    for instance in [
        serde_json::json!({}),
        serde_json::json!({"p": 1}),
        serde_json::json!({"p": "x"}),
        serde_json::json!({"q": 1}),
        serde_json::json!({"a": {}}),
        serde_json::json!({"a": {"p": 1}}),
        serde_json::json!(1),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
        );
    }
}

// A subschema applied inline and as a `$ref` target, with a reference below it.
#[jsonschema::validator(
    schema = r##"{"$id":"http://example.com/a.json","properties":{"foo":{"$id":"http://example.com/b.json","definitions":{"inner":{"properties":{"bar":{"type":"string"}}}},"allOf":[{"$ref":"#/definitions/inner"}]}},"$ref":"#/properties/foo/allOf/0"}"##
)]
struct InlineAndReferencedInEmbeddedResource;
#[jsonschema::validator(
    schema = r##"{"$id":"http://example.com/a.json","properties":{"foo":{"$id":"http://example.com/b.json","definitions":{"inner":{"properties":{"bar":{"type":"string"}}}},"allOf":[{"$ref":"#/definitions/inner"}]}},"allOf":[{"$ref":"#/properties/foo/allOf/0"}]}"##,
    draft = Draft7
)]
struct InlineAndReferencedInEmbeddedResourceDraft7;
#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/a","$defs":{"a":{"$ref":"#/$defs/a/properties/foo","properties":{"foo":{"properties":{"bar":{"$ref":"#/$defs/s"}}}}},"s":{"type":"string"}}}"##
)]
struct InlineAndReferencedBelowTarget;
#[jsonschema::validator(
    schema = r##"{"$ref":"#/$defs/a","$defs":{"a":{"allOf":[{"properties":{"bar":{"$ref":"#/$defs/s"}}}],"properties":{"foo":{"$ref":"#/$defs/a/allOf/0"}}},"s":{"type":"string"}}}"##
)]
struct ReferencedAfterInline;
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"object","$ref":"#/$defs/a","$defs":{"a":{"$ref":"#/$defs/a/properties/foo","properties":{"foo":{"properties":{"bar":{"$recursiveRef":"#"}}}}}}}"##,
    draft = Draft201909
)]
struct RecursiveRefInlineAndReferencedBelowTarget;

// Its errors report the evaluation path of the route that reached it.
#[test_case(InlineAndReferencedInEmbeddedResource::is_valid, InlineAndReferencedInEmbeddedResource::validate, InlineAndReferencedInEmbeddedResource::iter_errors, InlineAndReferencedInEmbeddedResource::evaluate, r##"{"$id":"http://example.com/a.json","properties":{"foo":{"$id":"http://example.com/b.json","definitions":{"inner":{"properties":{"bar":{"type":"string"}}}},"allOf":[{"$ref":"#/definitions/inner"}]}},"$ref":"#/properties/foo/allOf/0"}"##, Draft::Draft202012 ; "embedded_resource")]
#[test_case(InlineAndReferencedInEmbeddedResourceDraft7::is_valid, InlineAndReferencedInEmbeddedResourceDraft7::validate, InlineAndReferencedInEmbeddedResourceDraft7::iter_errors, InlineAndReferencedInEmbeddedResourceDraft7::evaluate, r##"{"$id":"http://example.com/a.json","properties":{"foo":{"$id":"http://example.com/b.json","definitions":{"inner":{"properties":{"bar":{"type":"string"}}}},"allOf":[{"$ref":"#/definitions/inner"}]}},"allOf":[{"$ref":"#/properties/foo/allOf/0"}]}"##, Draft::Draft7 ; "embedded_resource_draft7")]
#[test_case(InlineAndReferencedBelowTarget::is_valid, InlineAndReferencedBelowTarget::validate, InlineAndReferencedBelowTarget::iter_errors, InlineAndReferencedBelowTarget::evaluate, r##"{"$ref":"#/$defs/a","$defs":{"a":{"$ref":"#/$defs/a/properties/foo","properties":{"foo":{"properties":{"bar":{"$ref":"#/$defs/s"}}}}},"s":{"type":"string"}}}"##, Draft::Draft202012 ; "below_target")]
#[test_case(ReferencedAfterInline::is_valid, ReferencedAfterInline::validate, ReferencedAfterInline::iter_errors, ReferencedAfterInline::evaluate, r##"{"$ref":"#/$defs/a","$defs":{"a":{"allOf":[{"properties":{"bar":{"$ref":"#/$defs/s"}}}],"properties":{"foo":{"$ref":"#/$defs/a/allOf/0"}}},"s":{"type":"string"}}}"##, Draft::Draft202012 ; "referenced_after_inline")]
#[test_case(RecursiveRefInlineAndReferencedBelowTarget::is_valid, RecursiveRefInlineAndReferencedBelowTarget::validate, RecursiveRefInlineAndReferencedBelowTarget::iter_errors, RecursiveRefInlineAndReferencedBelowTarget::evaluate, r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","type":"object","$ref":"#/$defs/a","$defs":{"a":{"$ref":"#/$defs/a/properties/foo","properties":{"foo":{"properties":{"bar":{"$recursiveRef":"#"}}}}}}}"##, Draft::Draft201909 ; "recursive_ref_below_target")]
fn test_subschema_applied_inline_and_by_reference_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
    draft: Draft,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("valid schema");
    for instance in [
        serde_json::json!({"bar": 1, "foo": {"bar": 1}}),
        serde_json::json!({"bar": "a", "foo": {"bar": 1}}),
        serde_json::json!({"bar": 1, "foo": {"bar": "a"}}),
        serde_json::json!({"bar": "a", "foo": {"bar": "a"}}),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
        );
    }
}

const REF_IN_INLINE_RESOURCE: &str = r##"{"$id":"https://example.com/root","unevaluatedProperties":false,"allOf":[{"$id":"x","$ref":"#/$defs/t","$defs":{"t":{"properties":{"s":true}}}}],"$defs":{"t":{"properties":{"i":true}}}}"##;
const REF_IN_INLINE_RESOURCE_BRANCH: &str = r##"{"$id":"https://example.com/root","unevaluatedProperties":false,"anyOf":[{"$id":"x","properties":{"s":true},"allOf":[{"$ref":"#/$defs/t"}],"$defs":{"t":{"required":["s"]}}}],"$defs":{"t":{"required":["i"]}}}"##;
const REF_IN_INLINE_RESOURCE_DEPENDENT_SCHEMA: &str = r##"{"$id":"https://example.com/root","unevaluatedProperties":false,"dependentSchemas":{"s":{"$id":"x","$ref":"#/$defs/t","$defs":{"t":{"properties":{"s":true}}}}},"$defs":{"t":{"properties":{"i":true}}}}"##;
const REF_IN_INLINE_RESOURCE_PREFIX_ITEMS: &str = r##"{"$id":"https://example.com/root","unevaluatedItems":false,"allOf":[{"$id":"x","$ref":"#/$defs/t","$defs":{"t":{"prefixItems":[true,true]}}}],"$defs":{"t":{"prefixItems":[true]}}}"##;
const REF_IN_INLINE_RESOURCE_ITEMS: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","unevaluatedItems":false,"allOf":[{"$id":"x","$ref":"#/$defs/t","$defs":{"t":{"items":[true,true]}}}],"$defs":{"t":{"items":[true]}}}"##;
const DYNAMIC_REF_IN_INLINE_RESOURCE: &str = r##"{"$id":"https://example.com/root","unevaluatedProperties":false,"allOf":[{"$id":"x","$dynamicRef":"#t","$defs":{"t":{"$dynamicAnchor":"t","properties":{"s":true}}}}],"$defs":{"t":{"$anchor":"t","properties":{"i":true}}}}"##;
const RECURSIVE_REF_IN_INLINE_RESOURCE: &str = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$recursiveAnchor":true,"properties":{"o":true,"a":{"$id":"h","unevaluatedProperties":false,"allOf":[{"$id":"x","$recursiveAnchor":true,"$recursiveRef":"#"}]}}}"##;

#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","unevaluatedProperties":false,"allOf":[{"$id":"x","$ref":"#/$defs/t","$defs":{"t":{"properties":{"s":true}}}}],"$defs":{"t":{"properties":{"i":true}}}}"##
)]
struct RefInInlineResource;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","unevaluatedProperties":false,"anyOf":[{"$id":"x","properties":{"s":true},"allOf":[{"$ref":"#/$defs/t"}],"$defs":{"t":{"required":["s"]}}}],"$defs":{"t":{"required":["i"]}}}"##
)]
struct RefInInlineResourceBranch;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","unevaluatedProperties":false,"dependentSchemas":{"s":{"$id":"x","$ref":"#/$defs/t","$defs":{"t":{"properties":{"s":true}}}}},"$defs":{"t":{"properties":{"i":true}}}}"##
)]
struct RefInInlineResourceDependentSchema;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","unevaluatedItems":false,"allOf":[{"$id":"x","$ref":"#/$defs/t","$defs":{"t":{"prefixItems":[true,true]}}}],"$defs":{"t":{"prefixItems":[true]}}}"##
)]
struct RefInInlineResourcePrefixItems;
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","unevaluatedItems":false,"allOf":[{"$id":"x","$ref":"#/$defs/t","$defs":{"t":{"items":[true,true]}}}],"$defs":{"t":{"items":[true]}}}"##,
    draft = Draft201909
)]
struct RefInInlineResourceItems;
#[jsonschema::validator(
    schema = r##"{"$id":"https://example.com/root","unevaluatedProperties":false,"allOf":[{"$id":"x","$dynamicRef":"#t","$defs":{"t":{"$dynamicAnchor":"t","properties":{"s":true}}}}],"$defs":{"t":{"$anchor":"t","properties":{"i":true}}}}"##
)]
struct DynamicRefInInlineResource;
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"https://example.com/root","$recursiveAnchor":true,"properties":{"o":true,"a":{"$id":"h","unevaluatedProperties":false,"allOf":[{"$id":"x","$recursiveAnchor":true,"$recursiveRef":"#"}]}}}"##,
    draft = Draft201909
)]
struct RecursiveRefInInlineResource;

// A reference in a subschema with an `$id` resolves in that subschema's resource, also where
// `unevaluated*` counts what the subschema evaluates.
#[test_case(RefInInlineResource::is_valid, RefInInlineResource::validate, RefInInlineResource::iter_errors, RefInInlineResource::evaluate, REF_IN_INLINE_RESOURCE, serde_json::json!({"s": 1}), serde_json::json!({"i": 1}) ; "ref")]
#[test_case(RefInInlineResourceBranch::is_valid, RefInInlineResourceBranch::validate, RefInInlineResourceBranch::iter_errors, RefInInlineResourceBranch::evaluate, REF_IN_INLINE_RESOURCE_BRANCH, serde_json::json!({"s": 1}), serde_json::json!({"s": 1, "i": 1}) ; "ref in a branch")]
#[test_case(RefInInlineResourceDependentSchema::is_valid, RefInInlineResourceDependentSchema::validate, RefInInlineResourceDependentSchema::iter_errors, RefInInlineResourceDependentSchema::evaluate, REF_IN_INLINE_RESOURCE_DEPENDENT_SCHEMA, serde_json::json!({"s": 1}), serde_json::json!({"s": 1, "i": 1}) ; "ref in a dependent schema")]
#[test_case(RefInInlineResourcePrefixItems::is_valid, RefInInlineResourcePrefixItems::validate, RefInInlineResourcePrefixItems::iter_errors, RefInInlineResourcePrefixItems::evaluate, REF_IN_INLINE_RESOURCE_PREFIX_ITEMS, serde_json::json!([1, 2]), serde_json::json!([1, 2, 3]) ; "ref to prefix items")]
#[test_case(RefInInlineResourceItems::is_valid, RefInInlineResourceItems::validate, RefInInlineResourceItems::iter_errors, RefInInlineResourceItems::evaluate, REF_IN_INLINE_RESOURCE_ITEMS, serde_json::json!([1, 2]), serde_json::json!([1, 2, 3]) ; "ref to items")]
#[test_case(DynamicRefInInlineResource::is_valid, DynamicRefInInlineResource::validate, DynamicRefInInlineResource::iter_errors, DynamicRefInInlineResource::evaluate, DYNAMIC_REF_IN_INLINE_RESOURCE, serde_json::json!({"s": 1}), serde_json::json!({"i": 1}) ; "dynamic ref")]
#[test_case(RecursiveRefInInlineResource::is_valid, RecursiveRefInInlineResource::validate, RecursiveRefInInlineResource::iter_errors, RecursiveRefInInlineResource::evaluate, RECURSIVE_REF_IN_INLINE_RESOURCE, serde_json::json!({"a": {"o": 1}}), serde_json::json!({"a": {"z": 1}}) ; "recursive ref")]
fn test_reference_in_inline_resource_under_unevaluated_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: &str,
    valid: serde_json::Value,
    invalid: serde_json::Value,
) {
    let schema = serde_json::from_str(schema).expect("valid JSON");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for (instance, expected) in [(valid, true), (invalid, false)] {
        assert_eq!(runtime.is_valid(&instance), expected, "{instance}");
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
            "{instance}"
        );
    }
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

#[test_case(serde_json::json!(4), true ; "even number")]
#[test_case(serde_json::json!(5), false ; "odd number")]
#[test_case(serde_json::json!([2, 3]), false ; "odd item")]
#[test_case(serde_json::json!("5"), true ; "string")]
fn test_number_format(instance: serde_json::Value, expected: bool) {
    assert_eq!(EvaluateNumberFormat::is_valid(&instance), expected);
    assert_eq!(EvaluateNumberFormat::validate(&instance).is_ok(), expected);
    assert_eq!(
        EvaluateNumberFormat::iter_errors(&instance)
            .next()
            .is_none(),
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

#[test_case(serde_json::json!("2026-08-22T12:00:00Z"), true ; "well_formed_date_time")]
#[test_case(serde_json::json!("not-a-date"), false ; "malformed_date_time_rejected_by_assertion_vocab")]
fn test_format_assertion_vocabulary_parity(instance: serde_json::Value, expected: bool) {
    let schema = serde_json::json!({
        "$schema": "json-schema:///meta/format-assertion",
        "type": "string",
        "format": "date-time"
    });
    let runtime = build_runtime_with_resources(
        schema,
        [(
            "json-schema:///meta/format-assertion",
            serde_json::json!({
                "$id": "json-schema:///meta/format-assertion",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$vocabulary": {
                    "https://json-schema.org/draft/2020-12/vocab/core": true,
                    "https://json-schema.org/draft/2020-12/vocab/validation": true,
                    "https://json-schema.org/draft/2020-12/vocab/format-assertion": true
                }
            }),
        )],
    );

    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(
        FormatAssertionVocabularyValidator::is_valid(&instance),
        expected
    );
}

#[test_case(
    OptionalFormatAssertionVocabularyValidator::is_valid,
    "json-schema:///meta/optional-format-assertion",
    serde_json::json!({
        "$id": "json-schema:///meta/optional-format-assertion",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$vocabulary": {
            "https://json-schema.org/draft/2020-12/vocab/core": true,
            "https://json-schema.org/draft/2020-12/vocab/validation": true,
            "https://json-schema.org/draft/2020-12/vocab/format-assertion": false
        }
    })
    ; "optional_format_assertion_vocab"
)]
#[test_case(
    Draft201909FormatVocabularyValidator::is_valid,
    "json-schema:///meta/draft2019-format",
    serde_json::json!({
        "$id": "json-schema:///meta/draft2019-format",
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$vocabulary": {
            "https://json-schema.org/draft/2019-09/vocab/core": true,
            "https://json-schema.org/draft/2019-09/vocab/validation": true,
            "https://json-schema.org/draft/2019-09/vocab/format": true
        }
    })
    ; "draft2019_format_vocab_required"
)]
fn test_dialect_asserts_formats_parity(
    is_valid: fn(&serde_json::Value) -> bool,
    meta_uri: &'static str,
    meta: serde_json::Value,
) {
    let schema = serde_json::json!({
        "$schema": meta_uri,
        "type": "string",
        "format": "date-time"
    });
    let runtime = build_runtime_with_resources(schema, [(meta_uri, meta)]);

    for (instance, expected) in [
        (serde_json::json!("2026-08-22T12:00:00Z"), true),
        (serde_json::json!("not-a-date"), false),
    ] {
        assert_eq!(runtime.is_valid(&instance), expected);
        assert_eq!(is_valid(&instance), expected);
    }
}

#[test]
fn test_declared_vocabulary_compiles() {
    assert!(DeclaredVocabularyValidator::is_valid(&serde_json::json!(
        "text"
    )));
    assert!(!DeclaredVocabularyValidator::is_valid(&serde_json::json!(
        1
    )));
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

// `$ref` cycles in schemas that also resolve `$recursiveRef` or `$dynamicRef` through the dynamic
// scope: through nested instances, through the same instance, and to one target from two scopes.
#[jsonschema::validator(
    schema = r##"{"$recursiveAnchor": true, "properties": {"a": {"$recursiveRef": "#"}, "b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$ref": "#/$defs/node"}}}}"##,
    draft = Draft201909
)]
struct RefCycleBesideRecursiveRef;
#[jsonschema::validator(
    schema = r##"{"$dynamicAnchor": "meta", "properties": {"a": {"$dynamicRef": "#meta"}, "b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$ref": "#/$defs/node"}}}}"##,
    draft = Draft202012
)]
struct RefCycleBesideDynamicRef;
#[jsonschema::validator(
    schema = r##"{"$recursiveAnchor": true, "properties": {"a": {"$recursiveRef": "#"}, "b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minimum": 1, "allOf": [{"$ref": "#/$defs/node"}]}}}"##,
    draft = Draft201909
)]
struct SameInstanceRefCycleBesideRecursiveRef;
#[jsonschema::validator(
    schema = r##"{"$dynamicAnchor": "meta", "properties": {"a": {"$dynamicRef": "#meta"}, "b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minimum": 1, "allOf": [{"$ref": "#/$defs/node"}]}}}"##,
    draft = Draft202012
)]
struct SameInstanceRefCycleBesideDynamicRef;
#[jsonschema::validator(
    schema = r##"{"$id": "https://example.com/root", "allOf": [{"$ref": "strict"}, {"$ref": "tree"}], "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "$ref": "#/$defs/cycle", "$defs": {"cycle": {"minProperties": 1, "allOf": [{"$ref": "#/$defs/cycle"}], "additionalProperties": {"$recursiveRef": "#"}}}}, "strict": {"$id": "strict", "$recursiveAnchor": true, "$ref": "tree", "maxProperties": 1}}}"##,
    draft = Draft201909
)]
struct RefCycleUnderTwoRecursiveScopes;
#[jsonschema::validator(
    schema = r##"{"$id": "https://example.com/root", "allOf": [{"$ref": "strict"}, {"$ref": "tree"}], "$defs": {"tree": {"$id": "tree", "$dynamicAnchor": "node", "$ref": "#/$defs/cycle", "$defs": {"cycle": {"minProperties": 1, "allOf": [{"$ref": "#/$defs/cycle"}], "additionalProperties": {"$dynamicRef": "#node"}}}}, "strict": {"$id": "strict", "$dynamicAnchor": "node", "$ref": "tree", "maxProperties": 1}}}"##,
    draft = Draft202012
)]
struct RefCycleUnderTwoDynamicScopes;

#[test_case(RefCycleBesideRecursiveRef::is_valid, RefCycleBesideRecursiveRef::validate, RefCycleBesideRecursiveRef::iter_errors, RefCycleBesideRecursiveRef::evaluate, serde_json::json!({"$recursiveAnchor": true, "properties": {"a": {"$recursiveRef": "#"}, "b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$ref": "#/$defs/node"}}}}), Draft::Draft201909 ; "nested_instances_recursive_ref")]
#[test_case(RefCycleBesideDynamicRef::is_valid, RefCycleBesideDynamicRef::validate, RefCycleBesideDynamicRef::iter_errors, RefCycleBesideDynamicRef::evaluate, serde_json::json!({"$dynamicAnchor": "meta", "properties": {"a": {"$dynamicRef": "#meta"}, "b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$ref": "#/$defs/node"}}}}), Draft::Draft202012 ; "nested_instances_dynamic_ref")]
#[test_case(SameInstanceRefCycleBesideRecursiveRef::is_valid, SameInstanceRefCycleBesideRecursiveRef::validate, SameInstanceRefCycleBesideRecursiveRef::iter_errors, SameInstanceRefCycleBesideRecursiveRef::evaluate, serde_json::json!({"$recursiveAnchor": true, "properties": {"a": {"$recursiveRef": "#"}, "b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minimum": 1, "allOf": [{"$ref": "#/$defs/node"}]}}}), Draft::Draft201909 ; "same_instance_recursive_ref")]
#[test_case(SameInstanceRefCycleBesideDynamicRef::is_valid, SameInstanceRefCycleBesideDynamicRef::validate, SameInstanceRefCycleBesideDynamicRef::iter_errors, SameInstanceRefCycleBesideDynamicRef::evaluate, serde_json::json!({"$dynamicAnchor": "meta", "properties": {"a": {"$dynamicRef": "#meta"}, "b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minimum": 1, "allOf": [{"$ref": "#/$defs/node"}]}}}), Draft::Draft202012 ; "same_instance_dynamic_ref")]
#[test_case(RefCycleUnderTwoRecursiveScopes::is_valid, RefCycleUnderTwoRecursiveScopes::validate, RefCycleUnderTwoRecursiveScopes::iter_errors, RefCycleUnderTwoRecursiveScopes::evaluate, serde_json::json!({"$id": "https://example.com/root", "allOf": [{"$ref": "strict"}, {"$ref": "tree"}], "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "$ref": "#/$defs/cycle", "$defs": {"cycle": {"minProperties": 1, "allOf": [{"$ref": "#/$defs/cycle"}], "additionalProperties": {"$recursiveRef": "#"}}}}, "strict": {"$id": "strict", "$recursiveAnchor": true, "$ref": "tree", "maxProperties": 1}}}), Draft::Draft201909 ; "two_recursive_scopes")]
#[test_case(RefCycleUnderTwoDynamicScopes::is_valid, RefCycleUnderTwoDynamicScopes::validate, RefCycleUnderTwoDynamicScopes::iter_errors, RefCycleUnderTwoDynamicScopes::evaluate, serde_json::json!({"$id": "https://example.com/root", "allOf": [{"$ref": "strict"}, {"$ref": "tree"}], "$defs": {"tree": {"$id": "tree", "$dynamicAnchor": "node", "$ref": "#/$defs/cycle", "$defs": {"cycle": {"minProperties": 1, "allOf": [{"$ref": "#/$defs/cycle"}], "additionalProperties": {"$dynamicRef": "#node"}}}}, "strict": {"$id": "strict", "$dynamicAnchor": "node", "$ref": "tree", "maxProperties": 1}}}), Draft::Draft202012 ; "two_dynamic_scopes")]
fn test_ref_cycle_under_dynamic_scope_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    schema: serde_json::Value,
    draft: Draft,
) {
    let runtime = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("valid schema");
    for instance in [
        serde_json::json!({"b": {"x": {"y": 1}}}),
        serde_json::json!({"b": {"q": 1, "r": 2, "s": 3, "t": 4, "x": {"y": 1}}}),
        serde_json::json!({"b": 1}),
        serde_json::json!({"b": 0}),
        serde_json::json!({"a": {"b": {"x": 1}}}),
        serde_json::json!({"a": {"b": 0}}),
        serde_json::json!({"a": {"b": 1, "c": 2}}),
        serde_json::json!({"a": {"b": 1}}),
        serde_json::json!({"a": 1}),
        serde_json::json!({"a": "x"}),
        serde_json::json!({}),
        serde_json::json!(1),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
        // The evaluation tree of a same-instance cycle stops one visit before the runtime's.
        assert_eq!(
            evaluate(&instance).flag().valid,
            runtime.is_valid(&instance),
            "evaluate outcome mismatch for {instance}"
        );
    }
}

#[jsonschema::validator(
    schema = r##"{"$id": "https://example.com/root", "$ref": "start", "$defs": {"start": {"$id": "start", "$ref": "#/$defs/cycle", "$defs": {"cycle": {"properties": {"next": {"$ref": "wrap"}}, "additionalProperties": {"$dynamicRef": "leaf#node"}}}}, "leaf": {"$id": "leaf", "$dynamicAnchor": "node", "type": "integer"}, "wrap": {"$id": "wrap", "$dynamicAnchor": "node", "type": "object", "$ref": "start#/$defs/cycle"}}}"##,
    draft = Draft202012
)]
struct RefCycleReachedUnderAnotherDynamicAnchor;

// `cycle` is reached from `start`, then again below `wrap`, where its `$dynamicRef` resolves to
// `wrap`.
#[test_case(serde_json::json!({"next": {"a": 1}}) ; "rejected_below_wrap")]
#[test_case(serde_json::json!({"next": {"a": {}}}) ; "accepted_below_wrap")]
#[test_case(serde_json::json!({"next": {"a": {"b": 1}}}) ; "rejected_two_levels_below_wrap")]
#[test_case(serde_json::json!({"a": 1}) ; "accepted_outside_wrap")]
#[test_case(serde_json::json!({"a": "x"}) ; "rejected_outside_wrap")]
fn test_ref_cycle_reached_under_another_dynamic_anchor(instance: serde_json::Value) {
    let schema = serde_json::json!({"$id": "https://example.com/root", "$ref": "start", "$defs": {"start": {"$id": "start", "$ref": "#/$defs/cycle", "$defs": {"cycle": {"properties": {"next": {"$ref": "wrap"}}, "additionalProperties": {"$dynamicRef": "leaf#node"}}}}, "leaf": {"$id": "leaf", "$dynamicAnchor": "node", "type": "integer"}, "wrap": {"$id": "wrap", "$dynamicAnchor": "node", "type": "object", "$ref": "start#/$defs/cycle"}}});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        RefCycleReachedUnderAnotherDynamicAnchor::is_valid(&instance),
        RefCycleReachedUnderAnotherDynamicAnchor::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        RefCycleReachedUnderAnotherDynamicAnchor::iter_errors(&instance),
        &runtime,
        &instance,
    );
    assert_eq!(
        evaluation_outputs(&RefCycleReachedUnderAnotherDynamicAnchor::evaluate(
            &instance
        )),
        evaluation_outputs(&runtime.evaluate(&instance)),
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

// Draft 6 does not define the content keywords either.
#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-06/schema#","contentEncoding":"base64"}"#
)]
struct Draft6ContentEncodingIgnored;

#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-06/schema#","contentMediaType":"application/json"}"#
)]
struct Draft6ContentMediaTypeIgnored;

#[test_case(Draft6ContentEncodingIgnored::is_valid, serde_json::json!({"$schema":"http://json-schema.org/draft-06/schema#","contentEncoding":"base64"}), serde_json::json!("not base64!") ; "encoding")]
#[test_case(Draft6ContentMediaTypeIgnored::is_valid, serde_json::json!({"$schema":"http://json-schema.org/draft-06/schema#","contentMediaType":"application/json"}), serde_json::json!("{") ; "media_type")]
fn draft6_content_ignored_matches_runtime(
    is_valid: fn(&serde_json::Value) -> bool,
    schema: serde_json::Value,
    instance: serde_json::Value,
) {
    assert!(is_valid(&instance));
    assert_is_valid_parity(&schema, is_valid(&instance), &instance);
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

#[jsonschema::validator(schema = r#"{"type": "integer", "minLength": 1}"#)]
struct TypeExcludesString;
#[jsonschema::validator(schema = r#"{"type": "string", "minItems": 1}"#)]
struct TypeExcludesArray;
#[jsonschema::validator(schema = r#"{"type": "string", "minimum": 1}"#)]
struct TypeExcludesNumber;
#[jsonschema::validator(
    schema = r#"{"type": "integer", "properties": {"b": {"minimum": 5}}, "required": ["c"]}"#
)]
struct TypeExcludesObject;
#[jsonschema::validator(schema = r#"{"type": ["string", "null"], "properties": {"a": {}}}"#)]
struct TypeListExcludesObject;
#[jsonschema::validator(schema = r#"{"type": "object", "minLength": 1}"#)]
struct TypeObjectExcludesString;

type IsValidFn = fn(&serde_json::Value) -> bool;
type ValidateFn = for<'i> fn(&'i serde_json::Value) -> Result<(), jsonschema::ValidationError<'i>>;
type IterErrorsFn = for<'i> fn(&'i serde_json::Value) -> jsonschema::ErrorIterator<'i>;

// A `type` that leaves out the instance's type rejects it, even beside keywords for that type.
#[test_case(TypeExcludesString::is_valid, TypeExcludesString::validate, TypeExcludesString::iter_errors, serde_json::json!({"type": "integer", "minLength": 1}) ; "string_keywords")]
#[test_case(TypeExcludesArray::is_valid, TypeExcludesArray::validate, TypeExcludesArray::iter_errors, serde_json::json!({"type": "string", "minItems": 1}) ; "array_keywords")]
#[test_case(TypeExcludesNumber::is_valid, TypeExcludesNumber::validate, TypeExcludesNumber::iter_errors, serde_json::json!({"type": "string", "minimum": 1}) ; "number_keywords")]
#[test_case(TypeExcludesObject::is_valid, TypeExcludesObject::validate, TypeExcludesObject::iter_errors, serde_json::json!({"type": "integer", "properties": {"b": {"minimum": 5}}, "required": ["c"]}) ; "object_keywords")]
#[test_case(TypeListExcludesObject::is_valid, TypeListExcludesObject::validate, TypeListExcludesObject::iter_errors, serde_json::json!({"type": ["string", "null"], "properties": {"a": {}}}) ; "type_list_object_keywords")]
#[test_case(TypeObjectExcludesString::is_valid, TypeObjectExcludesString::validate, TypeObjectExcludesString::iter_errors, serde_json::json!({"type": "object", "minLength": 1}) ; "object_type_string_keywords")]
fn test_type_rejects_types_it_excludes(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    schema: serde_json::Value,
) {
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for instance in [
        serde_json::json!("abc"),
        serde_json::json!(""),
        serde_json::json!([1]),
        serde_json::json!(5),
        serde_json::json!(0),
        serde_json::json!({"b": 1}),
        serde_json::json!({"c": 1}),
        serde_json::json!(null),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
    }
}

#[jsonschema::validator(schema = r#"{"type": "string", "const": "a"}"#)]
struct TypeBesideConst;
#[jsonschema::validator(schema = r#"{"type": "string", "enum": ["a", "b"]}"#)]
struct TypeBesideEnum;
#[jsonschema::validator(schema = r#"{"type": "null", "const": null}"#)]
struct TypeBesideNullConst;

// `type` reports its own error even where `const` or `enum` already reject every other type.
#[test_case(TypeBesideConst::is_valid, TypeBesideConst::validate, TypeBesideConst::iter_errors, serde_json::json!({"type": "string", "const": "a"}) ; "const")]
#[test_case(TypeBesideEnum::is_valid, TypeBesideEnum::validate, TypeBesideEnum::iter_errors, serde_json::json!({"type": "string", "enum": ["a", "b"]}) ; "enum")]
#[test_case(TypeBesideNullConst::is_valid, TypeBesideNullConst::validate, TypeBesideNullConst::iter_errors, serde_json::json!({"type": "null", "const": null}) ; "null_const")]
fn test_type_beside_const_or_enum_reports_its_error(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    schema: serde_json::Value,
) {
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for instance in [
        serde_json::json!("a"),
        serde_json::json!("c"),
        serde_json::json!(2),
        serde_json::json!(null),
        serde_json::json!([]),
    ] {
        assert_validate_parity(
            is_valid(&instance),
            validate(&instance),
            &runtime,
            &instance,
        );
        assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
    }
}

#[jsonschema::validator(schema = r#"{"enum": []}"#)]
struct EmptyEnum;

// `enum: []` rejects every value; no other `enum` produces its message.
#[test]
fn test_empty_enum_message() {
    let runtime =
        jsonschema::validator_for(&serde_json::json!({"enum": []})).expect("valid schema");
    let instance = serde_json::json!(1);
    let generated_evaluation = EmptyEnum::evaluate(&instance);
    let runtime_evaluation = runtime.evaluate(&instance);
    let messages: Vec<String> = EmptyEnum::validate(&instance)
        .err()
        .into_iter()
        .chain(runtime.validate(&instance).err())
        .chain(EmptyEnum::iter_errors(&instance))
        .chain(runtime.iter_errors(&instance))
        .map(|error| error.to_string())
        .chain(
            generated_evaluation
                .iter_errors()
                .chain(runtime_evaluation.iter_errors())
                .map(|entry| entry.error.to_string()),
        )
        .collect();
    assert_eq!(messages, vec![r#"Empty "enum" does not allow 1"#; 6]);
}

#[jsonschema::validator(
    schema = r##"{"$defs": {"t": {"properties": {"a": {"type": "string"}}}}, "$ref": "#/$defs/t", "unevaluatedProperties": false}"##
)]
struct FailingReferenceProperties;
#[jsonschema::validator(
    schema = r##"{"$defs": {"t": {"$dynamicAnchor": "n", "properties": {"a": {"type": "string"}}}}, "$dynamicRef": "#n", "unevaluatedProperties": false}"##
)]
struct FailingDynamicReferenceProperties;
#[jsonschema::validator(
    schema = r#"{"if": true, "then": {"properties": {"a": {"type": "string"}}}, "unevaluatedProperties": false}"#
)]
struct FailingThenProperties;
#[jsonschema::validator(
    schema = r#"{"if": false, "else": {"properties": {"a": {"type": "string"}}}, "unevaluatedProperties": false}"#
)]
struct FailingElseProperties;
#[jsonschema::validator(
    schema = r#"{"dependentSchemas": {"a": {"properties": {"c": {"type": "string"}}}}, "unevaluatedProperties": false}"#
)]
struct FailingDependentSchemaProperties;
#[jsonschema::validator(
    schema = r#"{"allOf": [{"properties": {"a": {"type": "string"}}}], "unevaluatedProperties": false}"#
)]
struct FailingAllOfProperties;
#[jsonschema::validator(
    schema = r#"{"properties": {"a": {"type": "string"}}, "unevaluatedProperties": false}"#
)]
struct FailingPropertiesBeside;
#[jsonschema::validator(schema = r##"{"$defs": {"R": {"properties": {"foo": {"type": "integer"}, "child": {"$recursiveRef": "#", "unevaluatedProperties": false}}}}, "$ref": "#/$defs/R"}"##, draft = referencing::Draft::Draft201909)]
struct FailingRecursiveReferenceProperties;
#[jsonschema::validator(
    schema = r##"{"$defs": {"t": {"prefixItems": [{"type": "string"}]}}, "$ref": "#/$defs/t", "unevaluatedItems": false}"##
)]
struct FailingReferenceItems;
#[jsonschema::validator(
    schema = r##"{"$defs": {"t": {"$dynamicAnchor": "n", "prefixItems": [{"type": "string"}]}}, "$dynamicRef": "#n", "unevaluatedItems": false}"##
)]
struct FailingDynamicReferenceItems;
#[jsonschema::validator(
    schema = r#"{"if": true, "then": {"prefixItems": [{"type": "string"}]}, "unevaluatedItems": false}"#
)]
struct FailingThenItems;
#[jsonschema::validator(
    schema = r#"{"if": false, "else": {"prefixItems": [{"type": "string"}]}, "unevaluatedItems": false}"#
)]
struct FailingElseItems;
#[jsonschema::validator(
    schema = r##"{"$defs": {"t": {"contains": {"const": 1}, "minContains": 2}}, "$ref": "#/$defs/t", "unevaluatedItems": false}"##
)]
struct FailingContainsInReferenceItems;
#[jsonschema::validator(
    schema = r#"{"allOf": [{"prefixItems": [{"type": "string"}]}], "unevaluatedItems": false}"#
)]
struct FailingAllOfItems;
#[jsonschema::validator(
    schema = r#"{"prefixItems": [{"type": "string"}], "unevaluatedItems": false}"#
)]
struct FailingPrefixItemsBeside;
#[jsonschema::validator(schema = r##"{"$defs": {"R": {"items": [{"type": "string"}, {"$recursiveRef": "#", "unevaluatedItems": false}]}}, "$ref": "#/$defs/R"}"##, draft = referencing::Draft::Draft201909)]
struct FailingRecursiveReferenceItems;
#[jsonschema::validator(schema = r##"{"$recursiveAnchor": true, "$defs": {"t": {"properties": {"a": {"type": "string"}}}}, "$ref": "#/$defs/t", "properties": {"n": {"$recursiveRef": "#"}}, "unevaluatedProperties": false}"##, draft = referencing::Draft::Draft201909)]
struct FailingReferenceBesideRecursiveAnchorProperties;
#[jsonschema::validator(schema = r##"{"$recursiveAnchor": true, "$defs": {"t": {"items": [{}, {}, {"type": "string"}]}}, "$ref": "#/$defs/t", "items": [{"type": "string"}, {"$recursiveRef": "#"}], "unevaluatedItems": false}"##, draft = referencing::Draft::Draft201909)]
struct FailingReferenceBesideRecursiveAnchorItems;

// A subschema that fails evaluates nothing, so `unevaluated*` reports what it would have
// evaluated; keywords beside it evaluate theirs whether they pass or not.
#[test_case(FailingReferenceProperties::is_valid, FailingReferenceProperties::validate, FailingReferenceProperties::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"t": {"properties": {"a": {"type": "string"}}}}, "$ref": "#/$defs/t", "unevaluatedProperties": false}), serde_json::json!([{"a": 1, "b": 2}, {"a": 1}, {"a": "x", "b": 2}]) ; "reference_properties")]
#[test_case(FailingDynamicReferenceProperties::is_valid, FailingDynamicReferenceProperties::validate, FailingDynamicReferenceProperties::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"t": {"$dynamicAnchor": "n", "properties": {"a": {"type": "string"}}}}, "$dynamicRef": "#n", "unevaluatedProperties": false}), serde_json::json!([{"a": 1, "b": 2}]) ; "dynamic_reference_properties")]
#[test_case(FailingThenProperties::is_valid, FailingThenProperties::validate, FailingThenProperties::iter_errors, jsonschema::options(), serde_json::json!({"if": true, "then": {"properties": {"a": {"type": "string"}}}, "unevaluatedProperties": false}), serde_json::json!([{"a": 1, "b": 2}]) ; "then_properties")]
#[test_case(FailingElseProperties::is_valid, FailingElseProperties::validate, FailingElseProperties::iter_errors, jsonschema::options(), serde_json::json!({"if": false, "else": {"properties": {"a": {"type": "string"}}}, "unevaluatedProperties": false}), serde_json::json!([{"a": 1, "b": 2}]) ; "else_properties")]
#[test_case(FailingDependentSchemaProperties::is_valid, FailingDependentSchemaProperties::validate, FailingDependentSchemaProperties::iter_errors, jsonschema::options(), serde_json::json!({"dependentSchemas": {"a": {"properties": {"c": {"type": "string"}}}}, "unevaluatedProperties": false}), serde_json::json!([{"a": 1, "b": 3, "c": 2}]) ; "dependent_schema_properties")]
#[test_case(FailingAllOfProperties::is_valid, FailingAllOfProperties::validate, FailingAllOfProperties::iter_errors, jsonschema::options(), serde_json::json!({"allOf": [{"properties": {"a": {"type": "string"}}}], "unevaluatedProperties": false}), serde_json::json!([{"a": 1, "b": 2}]) ; "all_of_properties")]
#[test_case(FailingPropertiesBeside::is_valid, FailingPropertiesBeside::validate, FailingPropertiesBeside::iter_errors, jsonschema::options(), serde_json::json!({"properties": {"a": {"type": "string"}}, "unevaluatedProperties": false}), serde_json::json!([{"a": 1, "b": 2}]) ; "properties_beside")]
#[test_case(FailingRecursiveReferenceProperties::is_valid, FailingRecursiveReferenceProperties::validate, FailingRecursiveReferenceProperties::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$defs": {"R": {"properties": {"foo": {"type": "integer"}, "child": {"$recursiveRef": "#", "unevaluatedProperties": false}}}}, "$ref": "#/$defs/R"}), serde_json::json!([{"child": {"foo": "x"}}, {"child": {"child": {"bar": 1}}}]) ; "recursive_reference_properties")]
#[test_case(FailingReferenceItems::is_valid, FailingReferenceItems::validate, FailingReferenceItems::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"t": {"prefixItems": [{"type": "string"}]}}, "$ref": "#/$defs/t", "unevaluatedItems": false}), serde_json::json!([[1, 2], ["x", 2]]) ; "reference_items")]
#[test_case(FailingDynamicReferenceItems::is_valid, FailingDynamicReferenceItems::validate, FailingDynamicReferenceItems::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"t": {"$dynamicAnchor": "n", "prefixItems": [{"type": "string"}]}}, "$dynamicRef": "#n", "unevaluatedItems": false}), serde_json::json!([[1, 2]]) ; "dynamic_reference_items")]
#[test_case(FailingThenItems::is_valid, FailingThenItems::validate, FailingThenItems::iter_errors, jsonschema::options(), serde_json::json!({"if": true, "then": {"prefixItems": [{"type": "string"}]}, "unevaluatedItems": false}), serde_json::json!([[1, 2]]) ; "then_items")]
#[test_case(FailingElseItems::is_valid, FailingElseItems::validate, FailingElseItems::iter_errors, jsonschema::options(), serde_json::json!({"if": false, "else": {"prefixItems": [{"type": "string"}]}, "unevaluatedItems": false}), serde_json::json!([[1, 2]]) ; "else_items")]
#[test_case(FailingContainsInReferenceItems::is_valid, FailingContainsInReferenceItems::validate, FailingContainsInReferenceItems::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"t": {"contains": {"const": 1}, "minContains": 2}}, "$ref": "#/$defs/t", "unevaluatedItems": false}), serde_json::json!([[1, 2]]) ; "contains_in_reference_items")]
#[test_case(FailingAllOfItems::is_valid, FailingAllOfItems::validate, FailingAllOfItems::iter_errors, jsonschema::options(), serde_json::json!({"allOf": [{"prefixItems": [{"type": "string"}]}], "unevaluatedItems": false}), serde_json::json!([[1, 2]]) ; "all_of_items")]
#[test_case(FailingPrefixItemsBeside::is_valid, FailingPrefixItemsBeside::validate, FailingPrefixItemsBeside::iter_errors, jsonschema::options(), serde_json::json!({"prefixItems": [{"type": "string"}], "unevaluatedItems": false}), serde_json::json!([[1, 2]]) ; "prefix_items_beside")]
#[test_case(FailingRecursiveReferenceItems::is_valid, FailingRecursiveReferenceItems::validate, FailingRecursiveReferenceItems::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$defs": {"R": {"items": [{"type": "string"}, {"$recursiveRef": "#", "unevaluatedItems": false}]}}, "$ref": "#/$defs/R"}), serde_json::json!([["a", [1]]]) ; "recursive_reference_items")]
#[test_case(FailingReferenceBesideRecursiveAnchorProperties::is_valid, FailingReferenceBesideRecursiveAnchorProperties::validate, FailingReferenceBesideRecursiveAnchorProperties::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$recursiveAnchor": true, "$defs": {"t": {"properties": {"a": {"type": "string"}}}}, "$ref": "#/$defs/t", "properties": {"n": {"$recursiveRef": "#"}}, "unevaluatedProperties": false}), serde_json::json!([{"a": 1, "b": 2}, {"n": {"a": 1}}]) ; "reference_beside_recursive_anchor_properties")]
#[test_case(FailingReferenceBesideRecursiveAnchorItems::is_valid, FailingReferenceBesideRecursiveAnchorItems::validate, FailingReferenceBesideRecursiveAnchorItems::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$recursiveAnchor": true, "$defs": {"t": {"items": [{}, {}, {"type": "string"}]}}, "$ref": "#/$defs/t", "items": [{"type": "string"}, {"$recursiveRef": "#"}], "unevaluatedItems": false}), serde_json::json!([["a", [], 3], ["a", ["x"], "y"]]) ; "reference_beside_recursive_anchor_items")]
fn test_failing_subschema_evaluates_nothing(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    options: jsonschema::ValidationOptions,
    schema: serde_json::Value,
    instances: serde_json::Value,
) {
    let runtime = options.build(&schema).expect("valid schema");
    for instance in instances.as_array().expect("instance list") {
        assert_validate_parity(is_valid(instance), validate(instance), &runtime, instance);
        assert_iter_errors_parity(iter_errors(instance), &runtime, instance);
    }
}

#[jsonschema::validator(schema = r#"{"contains": {"type": "integer"}, "unevaluatedItems": false}"#, draft = Draft201909)]
struct ContainsBesideUnevaluatedItems201909;
#[jsonschema::validator(schema = r#"{"allOf": [{"contains": {"type": "integer"}}], "unevaluatedItems": false}"#, draft = Draft201909)]
struct ContainsInAllOfBesideUnevaluatedItems201909;
#[jsonschema::validator(schema = r##"{"$defs": {"c": {"contains": {"type": "integer"}}}, "$ref": "#/$defs/c", "unevaluatedItems": false}"##, draft = Draft201909)]
struct ContainsInReferenceBesideUnevaluatedItems201909;
#[jsonschema::validator(schema = r#"{"contains": {"type": "integer"}, "unevaluatedItems": false}"#, draft = Draft202012)]
struct ContainsBesideUnevaluatedItems202012;
#[jsonschema::validator(schema = r#"{"allOf": [{"contains": {"type": "integer"}}], "unevaluatedItems": false}"#, draft = Draft202012)]
struct ContainsInAllOfBesideUnevaluatedItems202012;
#[jsonschema::validator(schema = r##"{"$defs": {"c": {"contains": {"type": "integer"}}}, "$ref": "#/$defs/c", "unevaluatedItems": false}"##, draft = Draft202012)]
struct ContainsInReferenceBesideUnevaluatedItems202012;

// `contains` evaluates the items it matches from 2020-12 on, wherever it applies in place
#[test_case(ContainsBesideUnevaluatedItems201909::is_valid, ContainsBesideUnevaluatedItems201909::validate, ContainsBesideUnevaluatedItems201909::iter_errors, ContainsBesideUnevaluatedItems201909::evaluate, Draft::Draft201909, serde_json::json!({"contains": {"type": "integer"}, "unevaluatedItems": false}), false ; "2019_09_beside")]
#[test_case(ContainsInAllOfBesideUnevaluatedItems201909::is_valid, ContainsInAllOfBesideUnevaluatedItems201909::validate, ContainsInAllOfBesideUnevaluatedItems201909::iter_errors, ContainsInAllOfBesideUnevaluatedItems201909::evaluate, Draft::Draft201909, serde_json::json!({"allOf": [{"contains": {"type": "integer"}}], "unevaluatedItems": false}), false ; "2019_09_in_all_of")]
#[test_case(ContainsInReferenceBesideUnevaluatedItems201909::is_valid, ContainsInReferenceBesideUnevaluatedItems201909::validate, ContainsInReferenceBesideUnevaluatedItems201909::iter_errors, ContainsInReferenceBesideUnevaluatedItems201909::evaluate, Draft::Draft201909, serde_json::json!({"$defs": {"c": {"contains": {"type": "integer"}}}, "$ref": "#/$defs/c", "unevaluatedItems": false}), false ; "2019_09_in_reference")]
#[test_case(ContainsBesideUnevaluatedItems202012::is_valid, ContainsBesideUnevaluatedItems202012::validate, ContainsBesideUnevaluatedItems202012::iter_errors, ContainsBesideUnevaluatedItems202012::evaluate, Draft::Draft202012, serde_json::json!({"contains": {"type": "integer"}, "unevaluatedItems": false}), true ; "2020_12_beside")]
#[test_case(ContainsInAllOfBesideUnevaluatedItems202012::is_valid, ContainsInAllOfBesideUnevaluatedItems202012::validate, ContainsInAllOfBesideUnevaluatedItems202012::iter_errors, ContainsInAllOfBesideUnevaluatedItems202012::evaluate, Draft::Draft202012, serde_json::json!({"allOf": [{"contains": {"type": "integer"}}], "unevaluatedItems": false}), true ; "2020_12_in_all_of")]
#[test_case(ContainsInReferenceBesideUnevaluatedItems202012::is_valid, ContainsInReferenceBesideUnevaluatedItems202012::validate, ContainsInReferenceBesideUnevaluatedItems202012::iter_errors, ContainsInReferenceBesideUnevaluatedItems202012::evaluate, Draft::Draft202012, serde_json::json!({"$defs": {"c": {"contains": {"type": "integer"}}}, "$ref": "#/$defs/c", "unevaluatedItems": false}), true ; "2020_12_in_reference")]
fn test_contains_evaluates_items_from_2020_12(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    evaluate: EvaluateFn,
    draft: Draft,
    schema: serde_json::Value,
    expected: bool,
) {
    let runtime = jsonschema::options()
        .with_draft(draft)
        .build(&schema)
        .expect("valid schema");
    let instance = serde_json::json!([1]);
    assert_eq!(is_valid(&instance), expected);
    assert_validate_parity(
        is_valid(&instance),
        validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(iter_errors(&instance), &runtime, &instance);
    assert_evaluations_match(evaluate, &runtime);
}

#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/custom","contains":{"type":"integer"},"unevaluatedItems":false}"#,
    resources = {
        "json-schema:///meta/custom" => { schema = r#"{"$id":"json-schema:///meta/custom","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true,"https://json-schema.org/draft/2020-12/vocab/unevaluated":true}}"# },
    }
)]
struct ContainsBesideUnevaluatedItemsCustomMetaSchema;

// A `$schema` naming a custom meta-schema leaves the generated draft unknown, which reads
// `contains` as 2020-12 does
#[test_case(serde_json::json!([1]), true ; "matched_item")]
#[test_case(serde_json::json!([1, "x"]), false ; "unmatched_item")]
fn test_contains_evaluates_items_under_a_custom_meta_schema(
    instance: serde_json::Value,
    expected: bool,
) {
    let runtime = build_runtime_with_resources(
        serde_json::json!({
            "$schema": "json-schema:///meta/custom",
            "contains": {"type": "integer"},
            "unevaluatedItems": false
        }),
        [(
            "json-schema:///meta/custom",
            serde_json::json!({
                "$id": "json-schema:///meta/custom",
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$vocabulary": {
                    "https://json-schema.org/draft/2020-12/vocab/core": true,
                    "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                    "https://json-schema.org/draft/2020-12/vocab/validation": true,
                    "https://json-schema.org/draft/2020-12/vocab/unevaluated": true
                }
            }),
        )],
    );
    assert_eq!(
        ContainsBesideUnevaluatedItemsCustomMetaSchema::is_valid(&instance),
        expected
    );
    assert_validate_parity(
        ContainsBesideUnevaluatedItemsCustomMetaSchema::is_valid(&instance),
        ContainsBesideUnevaluatedItemsCustomMetaSchema::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        ContainsBesideUnevaluatedItemsCustomMetaSchema::iter_errors(&instance),
        &runtime,
        &instance,
    );
    assert_evaluations_match(
        ContainsBesideUnevaluatedItemsCustomMetaSchema::evaluate,
        &runtime,
    );
}

#[jsonschema::validator(schema = r#"{"type": "string", "minLength": 5, "const": "abcdefg"}"#)]
struct TypeBeforeConst;
#[jsonschema::validator(
    schema = r#"{"type": ["string", "null"], "minLength": 5, "enum": ["abcdefg"]}"#
)]
struct TypesBeforeEnum;
#[jsonschema::validator(schema = r#"{"type": "integer", "minimum": 5, "const": 7}"#)]
struct IntegerBeforeConst;
#[jsonschema::validator(
    schema = r#"{"type": "string", "minLength": 5, "allOf": [{"maxLength": 2}], "anyOf": [{"maxLength": 1}], "oneOf": [{"maxLength": 2}], "not": {"type": "string"}, "if": {"type": "string"}, "then": {"maxLength": 1}}"#
)]
struct LeafBeforeApplicators;
#[jsonschema::validator(
    schema = r##"{"$defs": {"a": {"maxLength": 1}}, "minLength": 5, "allOf": [{"maxLength": 2}], "$ref": "#/$defs/a"}"##
)]
struct ApplicatorsBeforeRef;
#[jsonschema::validator(
    schema = r##"{"$defs": {"a": {"required": ["x"]}}, "$ref": "#/$defs/a", "unevaluatedProperties": false}"##
)]
struct UnevaluatedBeforeRef;
#[jsonschema::validator(
    schema = r##"{"$defs": {"a": {"maxProperties": 0}}, "type": "object", "minProperties": 3, "allOf": [{"required": ["q"]}], "unevaluatedProperties": false, "$ref": "#/$defs/a"}"##
)]
struct ObjectPhases;
#[jsonschema::validator(
    schema = r#"{"type": "array", "minItems": 5, "allOf": [{"contains": {"type": "null"}}], "unevaluatedItems": false}"#
)]
struct ArrayPhases;
#[jsonschema::validator(
    schema = r##"{"$defs": {"a": {"prefixItems": [{"type": "string"}]}}, "maxItems": 1, "$ref": "#/$defs/a", "unevaluatedItems": false}"##
)]
struct ArrayUnevaluatedBeforeRef;
#[jsonschema::validator(
    schema = r#"{"type": "string", "allOf": [{"required": ["q"]}], "unevaluatedProperties": false}"#
)]
struct ExcludedTypeUnevaluated;
#[jsonschema::validator(
    schema = r#"{"dependentSchemas": {"a": {"required": ["q"]}}, "properties": {"a": {"type": "string"}}, "required": ["r"], "dependentRequired": {"a": ["s"]}}"#
)]
struct DependentKeywords;
#[jsonschema::validator(schema = r#"{"dependencies": {"a": ["s"]}, "properties": {"a": {"type": "string"}}}"#, draft = referencing::Draft::Draft7)]
struct DependenciesLast;
#[jsonschema::validator(schema = r#"{"minLength": 5, "maxLength": 1}"#)]
struct LengthBoundsBothFail;

// Errors come in the runtime's keyword order: `type`, `const` and `enum`, keywords for one type,
// `allOf` to `if`, `unevaluated*`, then references.
#[test_case(TypeBeforeConst::is_valid, TypeBeforeConst::validate, TypeBeforeConst::iter_errors, jsonschema::options(), serde_json::json!({"type": "string", "minLength": 5, "const": "abcdefg"}), serde_json::json!([2, "ab"]) ; "type_before_const")]
#[test_case(TypesBeforeEnum::is_valid, TypesBeforeEnum::validate, TypesBeforeEnum::iter_errors, jsonschema::options(), serde_json::json!({"type": ["string", "null"], "minLength": 5, "enum": ["abcdefg"]}), serde_json::json!([2, "ab"]) ; "types_before_enum")]
#[test_case(IntegerBeforeConst::is_valid, IntegerBeforeConst::validate, IntegerBeforeConst::iter_errors, jsonschema::options(), serde_json::json!({"type": "integer", "minimum": 5, "const": 7}), serde_json::json!([2.5, 2]) ; "integer_before_const")]
#[test_case(LeafBeforeApplicators::is_valid, LeafBeforeApplicators::validate, LeafBeforeApplicators::iter_errors, jsonschema::options(), serde_json::json!({"type": "string", "minLength": 5, "allOf": [{"maxLength": 2}], "anyOf": [{"maxLength": 1}], "oneOf": [{"maxLength": 2}], "not": {"type": "string"}, "if": {"type": "string"}, "then": {"maxLength": 1}}), serde_json::json!(["abc"]) ; "leaf_before_applicators")]
#[test_case(ApplicatorsBeforeRef::is_valid, ApplicatorsBeforeRef::validate, ApplicatorsBeforeRef::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"a": {"maxLength": 1}}, "minLength": 5, "allOf": [{"maxLength": 2}], "$ref": "#/$defs/a"}), serde_json::json!(["abc"]) ; "applicators_before_ref")]
#[test_case(UnevaluatedBeforeRef::is_valid, UnevaluatedBeforeRef::validate, UnevaluatedBeforeRef::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"a": {"required": ["x"]}}, "$ref": "#/$defs/a", "unevaluatedProperties": false}), serde_json::json!([{"b": 1}]) ; "unevaluated_before_ref")]
#[test_case(ObjectPhases::is_valid, ObjectPhases::validate, ObjectPhases::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"a": {"maxProperties": 0}}, "type": "object", "minProperties": 3, "allOf": [{"required": ["q"]}], "unevaluatedProperties": false, "$ref": "#/$defs/a"}), serde_json::json!([{"b": 1}]) ; "object_phases")]
#[test_case(ArrayPhases::is_valid, ArrayPhases::validate, ArrayPhases::iter_errors, jsonschema::options(), serde_json::json!({"type": "array", "minItems": 5, "allOf": [{"contains": {"type": "null"}}], "unevaluatedItems": false}), serde_json::json!([[1, 2]]) ; "array_phases")]
#[test_case(ArrayUnevaluatedBeforeRef::is_valid, ArrayUnevaluatedBeforeRef::validate, ArrayUnevaluatedBeforeRef::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"a": {"prefixItems": [{"type": "string"}]}}, "maxItems": 1, "$ref": "#/$defs/a", "unevaluatedItems": false}), serde_json::json!([[1, 2]]) ; "array_unevaluated_before_ref")]
#[test_case(ExcludedTypeUnevaluated::is_valid, ExcludedTypeUnevaluated::validate, ExcludedTypeUnevaluated::iter_errors, jsonschema::options(), serde_json::json!({"type": "string", "allOf": [{"required": ["q"]}], "unevaluatedProperties": false}), serde_json::json!([{"b": 1}]) ; "excluded_type_unevaluated")]
#[test_case(DependentKeywords::is_valid, DependentKeywords::validate, DependentKeywords::iter_errors, jsonschema::options(), serde_json::json!({"dependentSchemas": {"a": {"required": ["q"]}}, "properties": {"a": {"type": "string"}}, "required": ["r"], "dependentRequired": {"a": ["s"]}}), serde_json::json!([{"a": 1}]) ; "dependent_keywords")]
#[test_case(DependenciesLast::is_valid, DependenciesLast::validate, DependenciesLast::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft7), serde_json::json!({"dependencies": {"a": ["s"]}, "properties": {"a": {"type": "string"}}}), serde_json::json!([{"a": 1}]) ; "dependencies_last")]
#[test_case(LengthBoundsBothFail::is_valid, LengthBoundsBothFail::validate, LengthBoundsBothFail::iter_errors, jsonschema::options(), serde_json::json!({"minLength": 5, "maxLength": 1}), serde_json::json!(["abc"]) ; "length_bounds_both_fail")]
fn test_errors_follow_keyword_priority(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    options: jsonschema::ValidationOptions,
    schema: serde_json::Value,
    instances: serde_json::Value,
) {
    let runtime = options.build(&schema).expect("valid schema");
    for instance in instances.as_array().expect("instance list") {
        assert_validate_parity(is_valid(instance), validate(instance), &runtime, instance);
        assert_iter_errors_parity(iter_errors(instance), &runtime, instance);
    }
}

#[jsonschema::validator(
    schema = r##"{"anyOf": [{"required": ["a"], "allOf": [{"properties": {"a": true}}]}], "prefixItems": [true], "unevaluatedItems": false, "unevaluatedProperties": false}"##
)]
struct UnevaluatedBesideAnyOf;
#[jsonschema::validator(
    schema = r##"{"items": [{}], "additionalItems": {"patternProperties": {"^x-": {}}}}"##,
    draft = referencing::Draft::Draft7
)]
struct AdditionalItemsAcceptingEverything;
#[jsonschema::validator(
    schema = r##"{"properties": {"p": {"patternProperties": {"^a": {"patternProperties": {"^x-": {}}}}}, "q": {"prefixItems": [{"patternProperties": {"^x-": {}}}]}, "r": {"additionalProperties": {"patternProperties": {"^x-": {}}}}}, "additionalProperties": {"patternProperties": {"^x-": {}}}}"##
)]
struct SubschemasAcceptingEveryObject;
#[jsonschema::validator(
    schema = r##"{"properties": {"a": {"minimum": 1, "maximum": 10, "exclusiveMaximum": true}, "b": {"minimum": 1, "exclusiveMinimum": true, "maximum": 10}}}"##,
    draft = referencing::Draft::Draft4
)]
struct OneExclusiveBoundDraft4;
#[jsonschema::validator(
    schema = r##"{"$defs": {"a": {"$ref": "#/$defs/a", "properties": {"x": true}}}, "$ref": "#/$defs/a", "unevaluatedProperties": false}"##
)]
struct SelfReferenceBesideUnevaluatedProperties;
#[jsonschema::validator(
    schema = r##"{"properties": {"a": true}, "dependentSchemas": {"b": false}, "unevaluatedProperties": false}"##
)]
struct BooleanDependentSchemaBesideUnevaluatedProperties;
#[jsonschema::validator(
    schema = r##"{"$recursiveAnchor": true, "properties": {"child": {"$ref": "#"}}, "$recursiveRef": "#"}"##,
    draft = referencing::Draft::Draft201909
)]
struct RecursiveRefBesideSelfReference;
#[jsonschema::validator(
    schema = r##"{"properties": {"number": {"type": "number", "const": 5}, "integer": {"type": "integer", "const": 1}, "boolean": {"type": "boolean", "const": true}, "array": {"type": "array", "const": [1, 2]}, "none": {"type": "string", "enum": []}, "booleans": {"type": "boolean", "enum": [true, false]}, "null": {"type": "null", "enum": [null]}, "arrays": {"type": "array", "enum": [[1], [2]]}, "objects": {"type": "object", "enum": [{"a": 1}]}, "string_const_true": {"type": "string", "const": true}, "string_const_array": {"type": "string", "const": [1, 2]}, "string_enum_null": {"type": "string", "enum": [null]}, "string_enum_array": {"type": "string", "enum": [[1]]}, "string_enum_object": {"type": "string", "enum": [{"a": 1}]}}}"##
)]
struct TypeBesideConstOrEnum;

#[test_case(UnevaluatedBesideAnyOf::is_valid, UnevaluatedBesideAnyOf::validate, UnevaluatedBesideAnyOf::iter_errors, jsonschema::options(), serde_json::json!({"anyOf": [{"required": ["a"], "allOf": [{"properties": {"a": true}}]}], "prefixItems": [true], "unevaluatedItems": false, "unevaluatedProperties": false}), serde_json::json!([[1], [1, 2], {"a": 1}, {"a": 1, "b": 2}, {"b": 1}, "s"]) ; "unevaluated_items_and_properties_beside_any_of")]
#[test_case(AdditionalItemsAcceptingEverything::is_valid, AdditionalItemsAcceptingEverything::validate, AdditionalItemsAcceptingEverything::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft7), serde_json::json!({"items": [{}], "additionalItems": {"patternProperties": {"^x-": {}}}}), serde_json::json!([[1, {"x-a": 1}], [1, 2]]) ; "additional_items_accepting_everything")]
#[test_case(SubschemasAcceptingEveryObject::is_valid, SubschemasAcceptingEveryObject::validate, SubschemasAcceptingEveryObject::iter_errors, jsonschema::options(), serde_json::json!({"properties": {"p": {"patternProperties": {"^a": {"patternProperties": {"^x-": {}}}}}, "q": {"prefixItems": [{"patternProperties": {"^x-": {}}}]}, "r": {"additionalProperties": {"patternProperties": {"^x-": {}}}}}, "additionalProperties": {"patternProperties": {"^x-": {}}}}), serde_json::json!([{"p": {"ab": {"x-c": 1}}, "q": [{"x-a": 1}], "r": {"z": {}}, "s": {}}, 5]) ; "subschemas_accepting_every_object")]
#[test_case(OneExclusiveBoundDraft4::is_valid, OneExclusiveBoundDraft4::validate, OneExclusiveBoundDraft4::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft4), serde_json::json!({"properties": {"a": {"minimum": 1, "maximum": 10, "exclusiveMaximum": true}, "b": {"minimum": 1, "exclusiveMinimum": true, "maximum": 10}}}), serde_json::json!([{"a": 0}, {"a": 10}, {"a": 1}, {"b": 1}, {"b": 10}, {"b": 11}]) ; "one_exclusive_bound_draft4")]
#[test_case(SelfReferenceBesideUnevaluatedProperties::is_valid, SelfReferenceBesideUnevaluatedProperties::validate, SelfReferenceBesideUnevaluatedProperties::iter_errors, jsonschema::options(), serde_json::json!({"$defs": {"a": {"$ref": "#/$defs/a", "properties": {"x": true}}}, "$ref": "#/$defs/a", "unevaluatedProperties": false}), serde_json::json!([{"x": 1}, {"x": 1, "y": 2}, {}]) ; "self_reference_beside_unevaluated_properties")]
#[test_case(BooleanDependentSchemaBesideUnevaluatedProperties::is_valid, BooleanDependentSchemaBesideUnevaluatedProperties::validate, BooleanDependentSchemaBesideUnevaluatedProperties::iter_errors, jsonschema::options(), serde_json::json!({"properties": {"a": true}, "dependentSchemas": {"b": false}, "unevaluatedProperties": false}), serde_json::json!([{"a": 1}, {"a": 1, "c": 1}, {"b": 1}]) ; "boolean_dependent_schema_beside_unevaluated_properties")]
#[test_case(RecursiveRefBesideSelfReference::is_valid, RecursiveRefBesideSelfReference::validate, RecursiveRefBesideSelfReference::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$recursiveAnchor": true, "properties": {"child": {"$ref": "#"}}, "$recursiveRef": "#"}), serde_json::json!([{}, {"child": {}}, 5]) ; "recursive_ref_beside_self_reference")]
#[test_case(TypeBesideConstOrEnum::is_valid, TypeBesideConstOrEnum::validate, TypeBesideConstOrEnum::iter_errors, jsonschema::options(), serde_json::json!({"properties": {"number": {"type": "number", "const": 5}, "integer": {"type": "integer", "const": 1}, "boolean": {"type": "boolean", "const": true}, "array": {"type": "array", "const": [1, 2]}, "none": {"type": "string", "enum": []}, "booleans": {"type": "boolean", "enum": [true, false]}, "null": {"type": "null", "enum": [null]}, "arrays": {"type": "array", "enum": [[1], [2]]}, "objects": {"type": "object", "enum": [{"a": 1}]}, "string_const_true": {"type": "string", "const": true}, "string_const_array": {"type": "string", "const": [1, 2]}, "string_enum_null": {"type": "string", "enum": [null]}, "string_enum_array": {"type": "string", "enum": [[1]]}, "string_enum_object": {"type": "string", "enum": [{"a": 1}]}}}), serde_json::json!([{"number": 5}, {"number": "x"}, {"integer": 1}, {"integer": 2.5}, {"boolean": true}, {"boolean": 1}, {"array": [1, 2]}, {"array": "x"}, {"none": "a"}, {"none": 1}, {"booleans": false}, {"booleans": "x"}, {"null": null}, {"null": 0}, {"arrays": [1]}, {"arrays": {}}, {"objects": {"a": 1}}, {"objects": []}, {"string_const_true": true}, {"string_const_array": [1, 2]}, {"string_enum_null": null}, {"string_enum_array": [1]}, {"string_enum_object": {"a": 1}}]) ; "type_beside_const_or_enum")]
fn test_matches_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    options: jsonschema::ValidationOptions,
    schema: serde_json::Value,
    instances: serde_json::Value,
) {
    let runtime = options.build(&schema).expect("valid schema");
    for instance in instances.as_array().expect("instance list") {
        assert_validate_parity(is_valid(instance), validate(instance), &runtime, instance);
        assert_iter_errors_parity(iter_errors(instance), &runtime, instance);
    }
}

#[jsonschema::validator(
    schema = r##"{"properties": {"a": {"$id": "json-schema:///a.json", "$recursiveAnchor": true, "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}}"##,
    draft = referencing::Draft::Draft201909
)]
struct AnyOfBesideFailingKeywordUnderRecursiveRef;
#[jsonschema::validator(
    schema = r##"{"properties": {"a": {"$id": "json-schema:///a.json", "$recursiveAnchor": true, "propertyNames": {"pattern": "^q"}, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}}"##,
    draft = referencing::Draft::Draft201909
)]
struct AnyOfBesideFailingPropertyNamesUnderRecursiveRef;
#[jsonschema::validator(
    schema = r##"{"properties": {"a": {"$id": "json-schema:///a.json", "$recursiveAnchor": true, "minProperties": 5, "oneOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}}"##,
    draft = referencing::Draft::Draft201909
)]
struct OneOfBesideFailingKeywordUnderRecursiveRef;
#[jsonschema::validator(
    schema = r##"{"properties": {"a": {"$id": "http://example.com/a.json", "$recursiveAnchor": true, "minProperties": 5, "required": ["q"], "additionalProperties": {"$recursiveRef": "#"}}}}"##,
    draft = referencing::Draft::Draft201909
)]
struct RequiredBesideFailingKeywordUnderRecursiveRef;
#[jsonschema::validator(
    schema = r##"{"$recursiveAnchor": true, "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}"##,
    draft = referencing::Draft::Draft201909
)]
struct AnyOfBesideFailingKeywordUnderRootRecursiveRef;
#[jsonschema::validator(
    schema = r##"{"$defs": {"a": {"$id": "json-schema:///a.json", "$recursiveAnchor": true, "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}, "properties": {"a": {"$ref": "json-schema:///a.json"}}}"##,
    draft = referencing::Draft::Draft201909
)]
struct AnyOfBesideFailingKeywordUnderDefinitionRecursiveRef;
#[jsonschema::validator(
    schema = r##"{"$recursiveAnchor": true, "properties": {"a": {"$recursiveRef": "#"}, "p": {"properties": {"b": {"$id": "json-schema:///b.json", "$recursiveAnchor": true, "minProperties": 5, "required": ["q"]}}}}}"##,
    draft = referencing::Draft::Draft201909
)]
struct RequiredBesideFailingKeywordInNestedRecursiveAnchor;
#[jsonschema::validator(
    schema = r##"{"properties": {"a": {"$id": "json-schema:///a.json", "$dynamicAnchor": "node", "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$dynamicRef": "#node"}}}}"##,
    draft = referencing::Draft::Draft202012
)]
struct AnyOfBesideFailingKeywordUnderDynamicRef;
#[jsonschema::validator(
    schema = r##"{"properties": {"a": {"$id": "json-schema:///a.json", "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$ref": "#"}}}}"##,
    draft = referencing::Draft::Draft202012
)]
struct AnyOfBesideFailingKeywordUnderRefCycle;

// Subschemas reached in place or through a reference report every error, not only their first:
// `anyOf` and `oneOf` too when a keyword beside them already failed.
#[test_case(AnyOfBesideFailingKeywordUnderRecursiveRef::is_valid, AnyOfBesideFailingKeywordUnderRecursiveRef::validate, AnyOfBesideFailingKeywordUnderRecursiveRef::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"properties": {"a": {"$id": "json-schema:///a.json", "$recursiveAnchor": true, "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "any_of_under_recursive_ref")]
#[test_case(AnyOfBesideFailingPropertyNamesUnderRecursiveRef::is_valid, AnyOfBesideFailingPropertyNamesUnderRecursiveRef::validate, AnyOfBesideFailingPropertyNamesUnderRecursiveRef::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"properties": {"a": {"$id": "json-schema:///a.json", "$recursiveAnchor": true, "propertyNames": {"pattern": "^q"}, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "any_of_beside_property_names_under_recursive_ref")]
#[test_case(OneOfBesideFailingKeywordUnderRecursiveRef::is_valid, OneOfBesideFailingKeywordUnderRecursiveRef::validate, OneOfBesideFailingKeywordUnderRecursiveRef::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"properties": {"a": {"$id": "json-schema:///a.json", "$recursiveAnchor": true, "minProperties": 5, "oneOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "one_of_under_recursive_ref")]
#[test_case(RequiredBesideFailingKeywordUnderRecursiveRef::is_valid, RequiredBesideFailingKeywordUnderRecursiveRef::validate, RequiredBesideFailingKeywordUnderRecursiveRef::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"properties": {"a": {"$id": "http://example.com/a.json", "$recursiveAnchor": true, "minProperties": 5, "required": ["q"], "additionalProperties": {"$recursiveRef": "#"}}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "required_under_recursive_ref_in_resource")]
#[test_case(AnyOfBesideFailingKeywordUnderRootRecursiveRef::is_valid, AnyOfBesideFailingKeywordUnderRootRecursiveRef::validate, AnyOfBesideFailingKeywordUnderRootRecursiveRef::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$recursiveAnchor": true, "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}), serde_json::json!([{"x": {"y": 1}}]) ; "any_of_under_root_recursive_ref")]
#[test_case(AnyOfBesideFailingKeywordUnderDefinitionRecursiveRef::is_valid, AnyOfBesideFailingKeywordUnderDefinitionRecursiveRef::validate, AnyOfBesideFailingKeywordUnderDefinitionRecursiveRef::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$defs": {"a": {"$id": "json-schema:///a.json", "$recursiveAnchor": true, "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}, "properties": {"a": {"$ref": "json-schema:///a.json"}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "any_of_under_definition_recursive_ref")]
#[test_case(RequiredBesideFailingKeywordInNestedRecursiveAnchor::is_valid, RequiredBesideFailingKeywordInNestedRecursiveAnchor::validate, RequiredBesideFailingKeywordInNestedRecursiveAnchor::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft201909), serde_json::json!({"$recursiveAnchor": true, "properties": {"a": {"$recursiveRef": "#"}, "p": {"properties": {"b": {"$id": "json-schema:///b.json", "$recursiveAnchor": true, "minProperties": 5, "required": ["q"]}}}}}), serde_json::json!([{"p": {"b": {}}}, {"a": {"p": {"b": {}}}}]) ; "required_in_nested_recursive_anchor")]
#[test_case(AnyOfBesideFailingKeywordUnderDynamicRef::is_valid, AnyOfBesideFailingKeywordUnderDynamicRef::validate, AnyOfBesideFailingKeywordUnderDynamicRef::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft202012), serde_json::json!({"properties": {"a": {"$id": "json-schema:///a.json", "$dynamicAnchor": "node", "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$dynamicRef": "#node"}}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "any_of_under_dynamic_ref")]
#[test_case(AnyOfBesideFailingKeywordUnderRefCycle::is_valid, AnyOfBesideFailingKeywordUnderRefCycle::validate, AnyOfBesideFailingKeywordUnderRefCycle::iter_errors, jsonschema::options().with_draft(referencing::Draft::Draft202012), serde_json::json!({"properties": {"a": {"$id": "json-schema:///a.json", "minProperties": 5, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$ref": "#"}}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "any_of_under_ref_cycle")]
fn test_errors_beside_failing_keyword_match_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    options: jsonschema::ValidationOptions,
    schema: serde_json::Value,
    instances: serde_json::Value,
) {
    let runtime = options.build(&schema).expect("valid schema");
    for instance in instances.as_array().expect("instance list") {
        assert_validate_parity(is_valid(instance), validate(instance), &runtime, instance);
        assert_iter_errors_parity(iter_errors(instance), &runtime, instance);
    }
}

// Without the validation vocabulary `const` accepts every `kind`, so it cannot pick a branch.
#[jsonschema::validator(
    schema = r#"{"$schema": "json-schema:///meta/no-validation", "oneOf": [{"required": ["kind"], "properties": {"kind": {"const": "a"}}}, {"required": ["kind"], "properties": {"kind": {"const": "b"}}}]}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/no-validation" => { schema = r#"{"$id": "json-schema:///meta/no-validation", "$schema": "https://json-schema.org/draft/2020-12/schema", "$vocabulary": {"https://json-schema.org/draft/2020-12/vocab/core": true, "https://json-schema.org/draft/2020-12/vocab/applicator": true, "https://json-schema.org/draft/2020-12/vocab/validation": false, "https://json-schema.org/draft/2020-12/vocab/unevaluated": true, "https://json-schema.org/draft/2020-12/vocab/format-annotation": true}}"# },
    }
)]
struct NoValidationVocabularyOneOf;

#[test_case(serde_json::json!({"kind": "a"}) ; "first_branch_value")]
#[test_case(serde_json::json!({"kind": "c"}) ; "unknown_value")]
#[test_case(serde_json::json!({}) ; "no_discriminator")]
fn test_one_of_without_validation_vocabulary_matches_runtime(instance: serde_json::Value) {
    let runtime = build_runtime_with_resources(
        serde_json::json!({"$schema": "json-schema:///meta/no-validation", "oneOf": [{"required": ["kind"], "properties": {"kind": {"const": "a"}}}, {"required": ["kind"], "properties": {"kind": {"const": "b"}}}]}),
        [(
            "json-schema:///meta/no-validation",
            serde_json::json!({"$id": "json-schema:///meta/no-validation", "$schema": "https://json-schema.org/draft/2020-12/schema", "$vocabulary": {"https://json-schema.org/draft/2020-12/vocab/core": true, "https://json-schema.org/draft/2020-12/vocab/applicator": true, "https://json-schema.org/draft/2020-12/vocab/validation": false, "https://json-schema.org/draft/2020-12/vocab/unevaluated": true, "https://json-schema.org/draft/2020-12/vocab/format-annotation": true}}),
        )],
    );
    assert_eq!(
        NoValidationVocabularyOneOf::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(
    schema = r#"{"type": "string", "format": "empty"}"#,
    validate_formats = true,
    formats = {
        "empty" => ::core::primitive::str::is_empty,
    }
)]
struct CustomFormatAbsolutePath;

#[test_case(serde_json::json!("") ; "empty")]
#[test_case(serde_json::json!("a") ; "not_empty")]
fn test_custom_format_by_absolute_path_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::options()
        .should_validate_formats(true)
        .with_format("empty", str::is_empty)
        .build(&serde_json::json!({"type": "string", "format": "empty"}))
        .expect("valid schema");
    assert_eq!(
        CustomFormatAbsolutePath::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(schema = r#"{"type": "string", "minLength": 2}"#, backend = SerdeJson)]
struct ExplicitSerdeJsonBackend;

#[test_case(serde_json::json!("ab") ; "valid")]
#[test_case(serde_json::json!("a") ; "too_short")]
fn test_explicit_serde_json_backend_matches_runtime(instance: serde_json::Value) {
    let runtime = jsonschema::validator_for(&serde_json::json!({"type": "string", "minLength": 2}))
        .expect("valid schema");
    assert_validate_parity(
        ExplicitSerdeJsonBackend::is_valid(&instance),
        ExplicitSerdeJsonBackend::validate(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(schema = r#"{"propertyNames": {"maxLength": 0}}"#)]
struct PropertyNamesLeaf;
#[jsonschema::validator(
    schema = r##"{"$defs": {"s": {"maxLength": 0}}, "propertyNames": {"$ref": "#/$defs/s"}}"##
)]
struct PropertyNamesRef;
// The second hop resolves inside `other`, which defines its own `name`.
#[jsonschema::validator(
    schema = r##"{"$defs": {"name": {"type": "string"}, "other": {"$id": "http://example.com/other", "$defs": {"hop": {"$ref": "#/$defs/name"}, "name": {"maxLength": 1}}}}, "propertyNames": {"$ref": "http://example.com/other#/$defs/hop"}}"##
)]
struct PropertyNamesRefChainIntoResource;
#[jsonschema::validator(schema = r#"{"propertyNames": {"not": {"const": "a"}}}"#)]
struct PropertyNamesSubschema;
#[jsonschema::validator(
    schema = r#"{"patternProperties": {"^a": {}}, "additionalProperties": false, "propertyNames": {"pattern": "^a"}}"#
)]
struct PropertyNamesCovered;
#[jsonschema::validator(
    schema = r#"{"properties": {"a": {"type": "string"}}, "patternProperties": {"^a": {"type": "boolean"}}, "additionalProperties": false, "propertyNames": {"maxLength": 0}}"#
)]
struct PropertyNamesAfterApplicators;
#[jsonschema::validator(
    schema = r#"{"type": "object", "properties": {"x": {"propertyNames": {"minLength": 2}}}}"#
)]
struct NestedPropertyNames;
#[jsonschema::validator(
    schema = r#"{"$id": "http://example.com/root", "propertyNames": {"maxLength": 0}}"#
)]
struct PropertyNamesLeafInResource;
#[jsonschema::validator(
    schema = r#"{"$id": "http://example.com/root", "propertyNames": {"not": {"const": "a"}}}"#
)]
struct PropertyNamesSubschemaInResource;
#[jsonschema::validator(
    schema = r##"{"$id": "http://example.com/root", "$defs": {"s": {"maxLength": 0}}, "propertyNames": {"$ref": "#/$defs/s"}}"##
)]
struct PropertyNamesRefInResource;
#[jsonschema::validator(
    schema = r#"{"$id": "http://example.com/root", "$defs": {"other": {"$id": "http://example.com/other", "maxLength": 0}}, "propertyNames": {"$ref": "http://example.com/other"}}"#
)]
struct PropertyNamesRefIntoAnotherResource;
#[jsonschema::validator(
    schema = r#"{"$id": "http://example.com/root", "properties": {"x": {"$id": "http://example.com/x", "propertyNames": {"maxLength": 0}}}}"#
)]
struct PropertyNamesInEmbeddedResource;

// Each rejected name reports a `propertyNames` error wrapping the subschema's error, after the
// errors of `properties`, `patternProperties` and `additionalProperties`.
#[test_case(PropertyNamesLeaf::is_valid, PropertyNamesLeaf::validate, PropertyNamesLeaf::iter_errors, serde_json::json!({"propertyNames": {"maxLength": 0}}), serde_json::json!([{"a": 1, "bb": 2}]) ; "leaf")]
#[test_case(PropertyNamesRef::is_valid, PropertyNamesRef::validate, PropertyNamesRef::iter_errors, serde_json::json!({"$defs": {"s": {"maxLength": 0}}, "propertyNames": {"$ref": "#/$defs/s"}}), serde_json::json!([{"a": 1}]) ; "reference")]
#[test_case(PropertyNamesRefChainIntoResource::is_valid, PropertyNamesRefChainIntoResource::validate, PropertyNamesRefChainIntoResource::iter_errors, serde_json::json!({"$defs": {"name": {"type": "string"}, "other": {"$id": "http://example.com/other", "$defs": {"hop": {"$ref": "#/$defs/name"}, "name": {"maxLength": 1}}}}, "propertyNames": {"$ref": "http://example.com/other#/$defs/hop"}}), serde_json::json!([{"ab": 1}, {"a": 1}]) ; "reference_chain_into_another_resource")]
#[test_case(PropertyNamesSubschema::is_valid, PropertyNamesSubschema::validate, PropertyNamesSubschema::iter_errors, serde_json::json!({"propertyNames": {"not": {"const": "a"}}}), serde_json::json!([{"a": 1}]) ; "subschema")]
#[test_case(PropertyNamesCovered::is_valid, PropertyNamesCovered::validate, PropertyNamesCovered::iter_errors, serde_json::json!({"patternProperties": {"^a": {}}, "additionalProperties": false, "propertyNames": {"pattern": "^a"}}), serde_json::json!([{"b": 1}, {"a": 1}]) ; "covered_by_pattern_properties")]
#[test_case(PropertyNamesAfterApplicators::is_valid, PropertyNamesAfterApplicators::validate, PropertyNamesAfterApplicators::iter_errors, serde_json::json!({"properties": {"a": {"type": "string"}}, "patternProperties": {"^a": {"type": "boolean"}}, "additionalProperties": false, "propertyNames": {"maxLength": 0}}), serde_json::json!([{"a": 1, "bb": 2}]) ; "after_object_applicators")]
#[test_case(NestedPropertyNames::is_valid, NestedPropertyNames::validate, NestedPropertyNames::iter_errors, serde_json::json!({"type": "object", "properties": {"x": {"propertyNames": {"minLength": 2}}}}), serde_json::json!([{"x": {"a": 1}}]) ; "nested")]
#[test_case(PropertyNamesLeafInResource::is_valid, PropertyNamesLeafInResource::validate, PropertyNamesLeafInResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "propertyNames": {"maxLength": 0}}), serde_json::json!([{"a": 1, "bb": 2}]) ; "leaf_in_resource")]
#[test_case(PropertyNamesSubschemaInResource::is_valid, PropertyNamesSubschemaInResource::validate, PropertyNamesSubschemaInResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "propertyNames": {"not": {"const": "a"}}}), serde_json::json!([{"a": 1}]) ; "subschema_in_resource")]
#[test_case(PropertyNamesRefInResource::is_valid, PropertyNamesRefInResource::validate, PropertyNamesRefInResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "$defs": {"s": {"maxLength": 0}}, "propertyNames": {"$ref": "#/$defs/s"}}), serde_json::json!([{"a": 1}]) ; "reference_in_resource")]
#[test_case(PropertyNamesRefIntoAnotherResource::is_valid, PropertyNamesRefIntoAnotherResource::validate, PropertyNamesRefIntoAnotherResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "$defs": {"other": {"$id": "http://example.com/other", "maxLength": 0}}, "propertyNames": {"$ref": "http://example.com/other"}}), serde_json::json!([{"a": 1}]) ; "reference_into_another_resource")]
#[test_case(PropertyNamesInEmbeddedResource::is_valid, PropertyNamesInEmbeddedResource::validate, PropertyNamesInEmbeddedResource::iter_errors, serde_json::json!({"$id": "http://example.com/root", "properties": {"x": {"$id": "http://example.com/x", "propertyNames": {"maxLength": 0}}}}), serde_json::json!([{"x": {"a": 1}}]) ; "in_embedded_resource")]
fn test_property_names_errors_match_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    schema: serde_json::Value,
    instances: serde_json::Value,
) {
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for instance in instances.as_array().expect("instance list") {
        assert_validate_parity(is_valid(instance), validate(instance), &runtime, instance);
        assert_iter_errors_parity(iter_errors(instance), &runtime, instance);
    }
}

#[jsonschema::validator(
    schema = r##"{"$schema": "https://json-schema.org/draft/2020-12/schema", "$defs": {"s": {"type": "string"}}, "type": "array", "items": {"$dynamicRef": "#/$defs/s"}}"##
)]
struct DynamicRefItems;
#[jsonschema::validator(
    schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/main.json", "$defs": {"inner": {"$id": "inner.json", "$recursiveAnchor": true, "additionalProperties": {"$recursiveRef": "#"}}}, "if": {"propertyNames": {"pattern": "^[a-m]"}}, "then": {"$id": "leaf.json", "$recursiveAnchor": true, "$ref": "inner.json"}, "else": {"$id": "integer.json", "$recursiveAnchor": true, "$ref": "inner.json", "type": ["object", "integer"]}}"##
)]
struct RecursiveRefToBranch;
#[jsonschema::validator(
    schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/main.json", "properties": {"a": {"$id": "a.json", "$recursiveAnchor": true, "additionalProperties": {"$recursiveRef": "#"}}, "b": {"$id": "b.json", "$recursiveAnchor": true, "type": "object"}}}"##
)]
struct RecursiveAnchorInPlace;
#[jsonschema::validator(
    schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/main.json", "properties": {"a": {"$id": "a.json", "$recursiveAnchor": true, "propertyNames": {"maxLength": 0}, "additionalProperties": {"$recursiveRef": "#"}}}}"##
)]
struct RecursiveAnchorInPlacePropertyNames;
#[jsonschema::validator(
    schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/main.json", "properties": {"a": {"$id": "a.json", "$recursiveAnchor": true, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}}"##
)]
struct RecursiveAnchorInPlaceAnyOf;

// Errors reached through a reference carry the reference keyword in their evaluation path, and
// the location of their keyword in its own resource.
#[test_case(DynamicRefItems::is_valid, DynamicRefItems::validate, DynamicRefItems::iter_errors, serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$defs": {"s": {"type": "string"}}, "type": "array", "items": {"$dynamicRef": "#/$defs/s"}}), serde_json::json!([[1, "a", 2]]) ; "dynamic_reference_for_every_item")]
#[test_case(RecursiveRefToBranch::is_valid, RecursiveRefToBranch::validate, RecursiveRefToBranch::iter_errors, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/main.json", "$defs": {"inner": {"$id": "inner.json", "$recursiveAnchor": true, "additionalProperties": {"$recursiveRef": "#"}}}, "if": {"propertyNames": {"pattern": "^[a-m]"}}, "then": {"$id": "leaf.json", "$recursiveAnchor": true, "$ref": "inner.json"}, "else": {"$id": "integer.json", "$recursiveAnchor": true, "$ref": "inner.json", "type": ["object", "integer"]}}), serde_json::json!([{"november": 1.1}, {"november": {"x": 1.1}}]) ; "recursive_reference_to_a_branch")]
#[test_case(RecursiveAnchorInPlace::is_valid, RecursiveAnchorInPlace::validate, RecursiveAnchorInPlace::iter_errors, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/main.json", "properties": {"a": {"$id": "a.json", "$recursiveAnchor": true, "additionalProperties": {"$recursiveRef": "#"}}, "b": {"$id": "b.json", "$recursiveAnchor": true, "type": "object"}}}), serde_json::json!([{"b": 1}, {"a": {"x": 1}}]) ; "recursive_anchor_in_place")]
#[test_case(RecursiveAnchorInPlacePropertyNames::is_valid, RecursiveAnchorInPlacePropertyNames::validate, RecursiveAnchorInPlacePropertyNames::iter_errors, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/main.json", "properties": {"a": {"$id": "a.json", "$recursiveAnchor": true, "propertyNames": {"maxLength": 0}, "additionalProperties": {"$recursiveRef": "#"}}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "recursive_anchor_in_place_property_names")]
#[test_case(RecursiveAnchorInPlaceAnyOf::is_valid, RecursiveAnchorInPlaceAnyOf::validate, RecursiveAnchorInPlaceAnyOf::iter_errors, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/main.json", "properties": {"a": {"$id": "a.json", "$recursiveAnchor": true, "anyOf": [{"maxProperties": 0}, {"required": ["q"]}], "additionalProperties": {"$recursiveRef": "#"}}}}), serde_json::json!([{"a": {"x": {"y": 1}}}]) ; "recursive_anchor_in_place_any_of")]
fn test_error_locations_match_runtime(
    is_valid: IsValidFn,
    validate: ValidateFn,
    iter_errors: IterErrorsFn,
    schema: serde_json::Value,
    instances: serde_json::Value,
) {
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    for instance in instances.as_array().expect("instance list") {
        assert_validate_parity(is_valid(instance), validate(instance), &runtime, instance);
        assert_iter_errors_parity(iter_errors(instance), &runtime, instance);
    }
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

// Keywords in resources under the default base have no absolute location, even when a resource
// with one reaches them.
#[jsonschema::validator(
    schema = r##"{"$id": "http://example.com/root", "properties": {"a": {"$ref": "json-schema:///other"}, "b": {"$ref": "json-schema:///other#/$defs/s"}, "c": {"$ref": "json-schema:///never"}, "d": {"$id": "json-schema:///inner", "maxLength": 0}}}"##,
    resources = {
        "json-schema:///other" => { schema = r#"{"maxLength": 0, "$defs": {"s": {"minimum": 1}}}"# },
        "json-schema:///never" => { schema = "false" },
    }
)]
struct ResourcesUnderDefaultBase;

#[test_case(serde_json::json!({"a": "x"}) ; "reference")]
#[test_case(serde_json::json!({"b": 0}) ; "pointer below the resource root")]
#[test_case(serde_json::json!({"c": 1}) ; "false resource")]
#[test_case(serde_json::json!({"d": "x"}) ; "embedded resource")]
#[test_case(serde_json::json!({"a": "x", "b": 0, "c": 1, "d": "x"}) ; "all")]
fn test_resources_under_default_base_match_runtime(instance: serde_json::Value) {
    let runtime = build_runtime_with_resources(
        serde_json::json!({"$id": "http://example.com/root", "properties": {"a": {"$ref": "json-schema:///other"}, "b": {"$ref": "json-schema:///other#/$defs/s"}, "c": {"$ref": "json-schema:///never"}, "d": {"$id": "json-schema:///inner", "maxLength": 0}}}),
        [
            (
                "json-schema:///other",
                serde_json::json!({"maxLength": 0, "$defs": {"s": {"minimum": 1}}}),
            ),
            ("json-schema:///never", serde_json::json!(false)),
        ],
    );
    assert_validate_parity(
        ResourcesUnderDefaultBase::is_valid(&instance),
        ResourcesUnderDefaultBase::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        ResourcesUnderDefaultBase::iter_errors(&instance),
        &runtime,
        &instance,
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
#[test_case(serde_json::json!({"children":[{"children":[1]}]}) ; "deep_not_an_object")]
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
    assert_eq!(
        evaluation_outputs(&DynamicRefNoUnevaluatedVocab::evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
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
    assert!(!TypeNonStringArrayValue::evaluate(&instance).flag().valid);
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

impl<'i> jsonschema::Keyword<'i> for StringGateKeyword {
    fn validate(
        &self,
        instance: &'i serde_json::Value,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(jsonschema::ValidationError::custom("strings rejected"))
        }
    }

    fn is_valid(&self, instance: &'i serde_json::Value) -> bool {
        !instance.is_string() || self.allow_strings
    }
}

// The Result wrapping is required by the keyword factory signature.
#[allow(clippy::unnecessary_wraps)]
fn string_gate_factory<'a>(
    _parent: &'a serde_json::Map<String, serde_json::Value>,
    value: &'a serde_json::Value,
    _path: jsonschema::paths::Location,
) -> Result<Box<dyn for<'i> jsonschema::Keyword<'i>>, jsonschema::ValidationError<'a>> {
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

#[cfg(not(target_arch = "wasm32"))]
#[path = "generation/mod.rs"]
mod generation;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(
    schema = r##"{"$defs":{"circle":{"type":"object","required":["kind","radius"],"properties":{"kind":{"const":"circle"},"radius":{"type":"number","minimum":0}}},"square":{"type":"object","required":["kind","side"],"properties":{"kind":{"const":"square"},"side":{"type":"number","minimum":0}}}},"oneOf":[{"$ref":"#/$defs/circle"},{"$ref":"#/$defs/square"}]}"##
)]
struct ParityShapeValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(
    schema = r#"{"type":"array","minItems":3,"items":{"type":"integer","multipleOf":3}}"#
)]
struct ParityGridArrayValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(schema = r#"{"type":"string","minLength":5,"maxLength":8}"#)]
struct ParityWindowStringValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(
    schema = r#"{"anyOf":[{"type":"integer","minimum":10},{"type":"string","minLength":3}]}"#
)]
struct ParityAnyOfValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(schema = r#"{"type":"integer","minimum":100,"multipleOf":7}"#)]
struct ParitySparseIntegerValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(
    schema = r#"{"type":"object","required":["a"],"properties":{"a":{"type":"integer","minimum":0}},"additionalProperties":{"type":"string"}}"#
)]
struct ParityShieldedObjectValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(
    schema = r#"{"type":"object","patternProperties":{"^a":{"type":"integer"}},"propertyNames":{"maxLength":3}}"#
)]
struct ParityPatternObjectValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(
    schema = r#"{"type":"number","exclusiveMinimum":0,"multipleOf":0.5,"maximum":4}"#
)]
struct ParityFractionalGridValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(
    schema = r#"{"type":"array","prefixItems":[{"const":"tag"},{"type":"integer","minimum":1}],"items":{"type":"string","minLength":1}}"#
)]
struct ParityPrefixArrayValidator;

#[cfg(not(target_arch = "wasm32"))]
#[jsonschema::validator(schema = r#"{"enum":[[1,2],{"a":true},"x",3.5]}"#)]
struct ParityEnumValidator;

#[cfg(not(target_arch = "wasm32"))]
struct ParityTarget {
    source: &'static str,
    generated_is_valid: fn(&serde_json::Value) -> bool,
    generated_validate: fn(&serde_json::Value) -> Result<(), jsonschema::ValidationError<'_>>,
}

#[cfg(not(target_arch = "wasm32"))]
fn parity_targets() -> Vec<ParityTarget> {
    vec![
        ParityTarget {
            source: r##"{"$defs":{"circle":{"type":"object","required":["kind","radius"],"properties":{"kind":{"const":"circle"},"radius":{"type":"number","minimum":0}}},"square":{"type":"object","required":["kind","side"],"properties":{"kind":{"const":"square"},"side":{"type":"number","minimum":0}}}},"oneOf":[{"$ref":"#/$defs/circle"},{"$ref":"#/$defs/square"}]}"##,
            generated_is_valid: ParityShapeValidator::is_valid,
            generated_validate: ParityShapeValidator::validate,
        },
        ParityTarget {
            source: r#"{"type":"array","minItems":3,"items":{"type":"integer","multipleOf":3}}"#,
            generated_is_valid: ParityGridArrayValidator::is_valid,
            generated_validate: ParityGridArrayValidator::validate,
        },
        ParityTarget {
            source: r#"{"type":"string","minLength":5,"maxLength":8}"#,
            generated_is_valid: ParityWindowStringValidator::is_valid,
            generated_validate: ParityWindowStringValidator::validate,
        },
        ParityTarget {
            source: r#"{"anyOf":[{"type":"integer","minimum":10},{"type":"string","minLength":3}]}"#,
            generated_is_valid: ParityAnyOfValidator::is_valid,
            generated_validate: ParityAnyOfValidator::validate,
        },
        ParityTarget {
            source: r#"{"type":"integer","minimum":100,"multipleOf":7}"#,
            generated_is_valid: ParitySparseIntegerValidator::is_valid,
            generated_validate: ParitySparseIntegerValidator::validate,
        },
        ParityTarget {
            source: r#"{"type":"object","required":["a"],"properties":{"a":{"type":"integer","minimum":0}},"additionalProperties":{"type":"string"}}"#,
            generated_is_valid: ParityShieldedObjectValidator::is_valid,
            generated_validate: ParityShieldedObjectValidator::validate,
        },
        ParityTarget {
            source: r#"{"type":"object","patternProperties":{"^a":{"type":"integer"}},"propertyNames":{"maxLength":3}}"#,
            generated_is_valid: ParityPatternObjectValidator::is_valid,
            generated_validate: ParityPatternObjectValidator::validate,
        },
        ParityTarget {
            source: r#"{"type":"number","exclusiveMinimum":0,"multipleOf":0.5,"maximum":4}"#,
            generated_is_valid: ParityFractionalGridValidator::is_valid,
            generated_validate: ParityFractionalGridValidator::validate,
        },
        ParityTarget {
            source: r#"{"type":"array","prefixItems":[{"const":"tag"},{"type":"integer","minimum":1}],"items":{"type":"string","minLength":1}}"#,
            generated_is_valid: ParityPrefixArrayValidator::is_valid,
            generated_validate: ParityPrefixArrayValidator::validate,
        },
        ParityTarget {
            source: r#"{"enum":[[1,2],{"a":true},"x",3.5]}"#,
            generated_is_valid: ParityEnumValidator::is_valid,
            generated_validate: ParityEnumValidator::validate,
        },
    ]
}

// The generated validator agrees with the runtime one on values aimed inside each schema, where a
// blind draw almost never lands.
#[cfg(not(target_arch = "wasm32"))]
#[hegel::test(test_cases = 2_000)]
fn codegen_agrees_with_runtime_on_drawn_instances(tc: hegel::TestCase) {
    let targets = parity_targets();
    let index = tc.draw(
        hegel::generators::integers::<usize>()
            .min_value(0)
            .max_value(targets.len() - 1),
    );
    let target = &targets[index];
    let schema: serde_json::Value = serde_json::from_str(target.source).expect("valid schema JSON");
    let canonical = jsonschema::canonical::options()
        .canonicalize(&schema)
        .expect("the schema canonicalizes");
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    let modeled = jsonschema::validator_for(&canonical.to_json_schema())
        .expect("the canonical form compiles");
    let mut values = vec![tc.draw(generation::arbitrary_instance())];
    if let Some(instance) = generation::draw_valid_instance(&tc, &canonical, &modeled) {
        values.push(instance);
    }
    for value in &values {
        assert_validate_parity(
            (target.generated_is_valid)(value),
            (target.generated_validate)(value),
            &runtime,
            value,
        );
    }
}

// `methods` drops generated methods; the ones that remain must still compile against the shared
// helpers and agree with the runtime validator. `anyOf` exercises the branch helpers, `$ref` the
// per-target helpers, `$ref: "#"` the recursive stacks.
#[jsonschema::validator(
    schema = r##"{"type":"object","properties":{"a":{"anyOf":[{"type":"string"},{"type":"number"}]},"b":{"$ref":"#/properties/a"}}}"##,
    methods = { is_valid = false }
)]
struct MethodsIsValidOff;

#[jsonschema::validator(
    schema = r##"{"type":"object","properties":{"a":{"anyOf":[{"type":"string"},{"type":"number"}]},"b":{"$ref":"#/properties/a"}}}"##,
    methods = { validate = false }
)]
struct MethodsValidateOff;

#[jsonschema::validator(
    schema = r##"{"type":"object","properties":{"a":{"anyOf":[{"type":"string"},{"type":"number"}]},"b":{"$ref":"#/properties/a"}}}"##,
    methods = { iter_errors = false }
)]
struct MethodsIterErrorsOff;

#[jsonschema::validator(
    schema = r##"{"type":"object","properties":{"a":{"anyOf":[{"type":"string"},{"type":"number"}]},"b":{"$ref":"#/properties/a"}}}"##,
    methods = { validate = false, iter_errors = false }
)]
struct MethodsOnlyIsValid;

fn methods_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{"a":{"anyOf":[{"type":"string"},{"type":"number"}]},"b":{"$ref":"#/properties/a"}}})
}

#[test_case(serde_json::json!({"a":"x","b":1}) ; "valid")]
#[test_case(serde_json::json!({"a":[],"b":true}) ; "invalid_both")]
#[test_case(serde_json::json!("text") ; "non_object")]
fn test_methods_subsets(instance: serde_json::Value) {
    let runtime = jsonschema::validator_for(&methods_schema()).expect("valid schema");
    assert_validate_parity(
        runtime.is_valid(&instance),
        MethodsIsValidOff::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        MethodsIsValidOff::iter_errors(&instance),
        &runtime,
        &instance,
    );
    assert_eq!(
        MethodsValidateOff::is_valid(&instance),
        runtime.is_valid(&instance)
    );
    assert_iter_errors_parity(
        MethodsValidateOff::iter_errors(&instance),
        &runtime,
        &instance,
    );
    assert_validate_parity(
        MethodsIterErrorsOff::is_valid(&instance),
        MethodsIterErrorsOff::validate(&instance),
        &runtime,
        &instance,
    );
    assert_eq!(
        MethodsOnlyIsValid::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

#[jsonschema::validator(
    schema = r##"{"type":"object","properties":{"child":{"$ref":"#"},"kind":{"anyOf":[{"type":"string"},{"type":"integer"}]}}}"##,
    methods = { validate = false }
)]
struct MethodsRecursiveValidateOff;

#[jsonschema::validator(
    schema = r##"{"type":"object","properties":{"child":{"$ref":"#"},"kind":{"anyOf":[{"type":"string"},{"type":"integer"}]}}}"##,
    methods = { validate = false, iter_errors = false }
)]
struct MethodsRecursiveOnlyIsValid;

fn methods_recursive_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{"child":{"$ref":"#"},"kind":{"anyOf":[{"type":"string"},{"type":"integer"}]}}})
}

#[test_case(serde_json::json!({"kind":"x","child":{"kind":3}}) ; "valid")]
#[test_case(serde_json::json!({"kind":"x","child":{"kind":[]}}) ; "invalid_nested")]
fn test_methods_recursive_subsets(instance: serde_json::Value) {
    let runtime = jsonschema::validator_for(&methods_recursive_schema()).expect("valid schema");
    assert_eq!(
        MethodsRecursiveValidateOff::is_valid(&instance),
        runtime.is_valid(&instance)
    );
    assert_iter_errors_parity(
        MethodsRecursiveValidateOff::iter_errors(&instance),
        &runtime,
        &instance,
    );
    assert_eq!(
        MethodsRecursiveOnlyIsValid::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

// `$recursiveRef` and `$dynamicRef` push per-target `validate`/`collect` helpers onto their
// stacks; with those helpers dropped the pushes must go too.
#[jsonschema::validator(
    schema = r##"{"$id":"https://ex/m","$recursiveAnchor":true,"type":["object","boolean"],"properties":{"allOf":{"type":"array","items":{"$recursiveRef":"#"}}}}"##,
    draft = Draft201909,
    methods = { validate = false, iter_errors = false }
)]
struct MethodsRecursiveRefOnlyIsValid;

#[jsonschema::validator(
    schema = r##"{"$id":"https://ex/d","$dynamicAnchor":"node","type":["object","boolean"],"properties":{"allOf":{"type":"array","items":{"$dynamicRef":"#node"}}}}"##,
    draft = Draft202012,
    methods = { validate = false, iter_errors = false }
)]
struct MethodsDynamicRefOnlyIsValid;

#[test_case(serde_json::json!({"allOf":[true, {"allOf":[false]}]}) ; "valid")]
#[test_case(serde_json::json!({"allOf":[true, {"allOf":[1]}]}) ; "invalid_nested")]
fn test_methods_stack_refs_only_is_valid(instance: serde_json::Value) {
    let recursive = serde_json::json!({"$id":"https://ex/m","$recursiveAnchor":true,"type":["object","boolean"],"properties":{"allOf":{"type":"array","items":{"$recursiveRef":"#"}}}});
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft201909)
        .build(&recursive)
        .expect("valid schema");
    assert_eq!(
        MethodsRecursiveRefOnlyIsValid::is_valid(&instance),
        runtime.is_valid(&instance)
    );
    let dynamic = serde_json::json!({"$id":"https://ex/d","$dynamicAnchor":"node","type":["object","boolean"],"properties":{"allOf":{"type":"array","items":{"$dynamicRef":"#node"}}}});
    let runtime = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&dynamic)
        .expect("valid schema");
    assert_eq!(
        MethodsDynamicRefOnlyIsValid::is_valid(&instance),
        runtime.is_valid(&instance)
    );
}

// `oneOf` keeps its `is_branch_valid_*` gates when the branch error collectors are dropped.
#[jsonschema::validator(
    schema = r#"{"oneOf":[{"properties":{"a":{}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}]}"#,
    methods = { validate = false, iter_errors = false }
)]
struct MethodsOneOfOnlyIsValid;

#[test_case(serde_json::json!({"a":1}) ; "one_branch")]
#[test_case(serde_json::json!({"a":1,"b":2}) ; "both_branches")]
#[test_case(serde_json::json!({}) ; "no_branch")]
fn test_methods_one_of_only_is_valid(instance: serde_json::Value) {
    let schema = serde_json::json!({"oneOf":[{"properties":{"a":{}},"required":["a"]},{"properties":{"b":{}},"required":["b"]}]});
    assert_is_valid_parity(
        &schema,
        MethodsOneOfOnlyIsValid::is_valid(&instance),
        &instance,
    );
}

#[jsonschema::validator(
    schema = r#"{"not":{"pattern":"(?<=ab)c"}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedNotOverMatchValidator;

#[jsonschema::validator(
    schema = r#"{"not":{"pattern":"(?<=x)c"}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedNotOverMissValidator;

#[jsonschema::validator(
    schema = r#"{"if":{"pattern":"(?<=ab)c"},"then":{"maxLength":1}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedIfPicksBranchValidator;

#[jsonschema::validator(
    schema = r#"{"oneOf":[{"pattern":"(?<=ab)c"},{"type":"string"}]}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedOneOfCountsValidator;

#[jsonschema::validator(
    schema = r#"{"patternProperties":{"(?<=ab)c":{"type":"integer"}}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedPatternPropertiesValidator;

#[jsonschema::validator(
    schema = r#"{"patternProperties":{"(?<=ab)c":true},"additionalProperties":false}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedAdditionalPropertiesValidator;

#[jsonschema::validator(
    schema = r#"{"patternProperties":{"(?<=ab)c":true},"unevaluatedProperties":false}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedUnevaluatedPropertiesValidator;

#[jsonschema::validator(
    schema = r#"{"properties":{"abc":true},"patternProperties":{"(?<=ab)c":{"type":"integer"}},"additionalProperties":false}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedDeclaredAdditionalFalseValidator;

#[jsonschema::validator(
    schema = r#"{"properties":{"abc":true},"patternProperties":{"(?<=ab)c":{"type":"integer"}},"additionalProperties":{}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedDeclaredAdditionalSchemaValidator;

#[jsonschema::validator(
    schema = r#"{"anyOf":[{"pattern":"(?<=ab)c"},{"type":"string"}]}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedAnyOfAcceptingValidator;

#[jsonschema::validator(
    schema = r#"{"anyOf":[{"pattern":"(?<=ab)c"},{"not":{"pattern":"(?<=ab)c"}}]}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedAnyOfNegationValidator;

#[jsonschema::validator(
    schema = r#"{"if":{"pattern":"(?<=ab)c"},"then":{"minLength":1},"else":{"minLength":1}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedIfEqualBranchesValidator;

#[jsonschema::validator(
    schema = r#"{"pattern":"(?<=ab)c","maxLength":1}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedSharedErrorValidator;

// With `backtrack_limit = 1` the engine cannot finish matching these patterns against "abc"
const UNDECIDED: &str = "Error executing regex: Max limit for backtracking count exceeded";

fn undecided() -> (bool, Result<(), String>, Vec<String>) {
    (false, Err(UNDECIDED.to_owned()), vec![UNDECIDED.to_owned()])
}

fn shared_max_length_error() -> (bool, Result<(), String>, Vec<String>) {
    let error = r#""abc" is longer than 1 character"#.to_owned();
    (false, Err(error.clone()), vec![error])
}

macro_rules! unfinished_match {
    ($name:ident, $validator:ty, $instance:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let instance = $instance;
            assert_eq!(
                (
                    <$validator>::is_valid(&instance),
                    <$validator>::validate(&instance).map_err(|error| error.to_string()),
                    <$validator>::iter_errors(&instance)
                        .map(|error| error.to_string())
                        .collect::<Vec<_>>(),
                ),
                $expected
            );
        }
    };
}

unfinished_match!(
    test_unfinished_not_over_match,
    UnfinishedNotOverMatchValidator,
    serde_json::json!("abc"),
    undecided()
);
unfinished_match!(
    test_unfinished_not_over_miss,
    UnfinishedNotOverMissValidator,
    serde_json::json!("abc"),
    undecided()
);
unfinished_match!(
    test_unfinished_if_picks_branch,
    UnfinishedIfPicksBranchValidator,
    serde_json::json!("abc"),
    undecided()
);
unfinished_match!(
    test_unfinished_one_of_counts,
    UnfinishedOneOfCountsValidator,
    serde_json::json!("abc"),
    undecided()
);
unfinished_match!(
    test_unfinished_pattern_properties,
    UnfinishedPatternPropertiesValidator,
    serde_json::json!({"abc":"x"}),
    undecided()
);
unfinished_match!(
    test_unfinished_additional_properties,
    UnfinishedAdditionalPropertiesValidator,
    serde_json::json!({"abc":1}),
    undecided()
);
unfinished_match!(
    test_unfinished_unevaluated_properties,
    UnfinishedUnevaluatedPropertiesValidator,
    serde_json::json!({"abc":1}),
    undecided()
);
unfinished_match!(
    test_unfinished_declared_additional_false,
    UnfinishedDeclaredAdditionalFalseValidator,
    serde_json::json!({"abc":"x"}),
    undecided()
);
unfinished_match!(
    test_unfinished_declared_additional_schema,
    UnfinishedDeclaredAdditionalSchemaValidator,
    serde_json::json!({"abc":"x"}),
    undecided()
);
unfinished_match!(
    test_unfinished_any_of_accepting,
    UnfinishedAnyOfAcceptingValidator,
    serde_json::json!("abc"),
    (true, Ok(()), Vec::new())
);
unfinished_match!(
    test_unfinished_any_of_negation,
    UnfinishedAnyOfNegationValidator,
    serde_json::json!("abc"),
    (true, Ok(()), Vec::new())
);
unfinished_match!(
    test_unfinished_if_equal_branches,
    UnfinishedIfEqualBranchesValidator,
    serde_json::json!("abc"),
    (true, Ok(()), Vec::new())
);
unfinished_match!(
    test_unfinished_shared_error,
    UnfinishedSharedErrorValidator,
    serde_json::json!("abc"),
    shared_max_length_error()
);

#[jsonschema::validator(
    schema = r#"{"properties":{"x":{"pattern":"(?<=ab)c"}}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedNestedPatternValidator;

#[jsonschema::validator(
    schema = r##"{"$defs":{"s":{"pattern":"(?<=ab)c"}},"properties":{"x":{"$ref":"#/$defs/s"}}}"##,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedReferencedPatternValidator;

macro_rules! unfinished_match_location {
    ($name:ident, $validator:ty, $schema:literal, $instance:expr) => {
        #[test]
        fn $name() {
            let schema: serde_json::Value =
                serde_json::from_str($schema).expect("valid schema json");
            let runtime = jsonschema::options()
                .with_pattern_options(jsonschema::PatternOptions::fancy_regex().backtrack_limit(1))
                .build(&schema)
                .expect("valid schema");
            let instance = $instance;
            let locations = |error: jsonschema::ValidationError<'_>| {
                (
                    error.schema_path().to_string(),
                    error.instance_path().to_string(),
                    error.instance().clone().into_owned(),
                    error.to_string(),
                )
            };
            assert_eq!(
                <$validator>::validate(&instance).map_err(locations),
                runtime.validate(&instance).map_err(locations)
            );
        }
    };
}

unfinished_match_location!(
    test_unfinished_match_location_not,
    UnfinishedNotOverMatchValidator,
    r#"{"not":{"pattern":"(?<=ab)c"}}"#,
    serde_json::json!("abc")
);
unfinished_match_location!(
    test_unfinished_match_location_pattern,
    UnfinishedNestedPatternValidator,
    r#"{"properties":{"x":{"pattern":"(?<=ab)c"}}}"#,
    serde_json::json!({"x":"abc"})
);
unfinished_match_location!(
    test_unfinished_match_location_pattern_behind_reference,
    UnfinishedReferencedPatternValidator,
    r##"{"$defs":{"s":{"pattern":"(?<=ab)c"}},"properties":{"x":{"$ref":"#/$defs/s"}}}"##,
    serde_json::json!({"x":"abc"})
);

// 30 `a`s and a `!` need more than 5,000 backtracking steps for this pattern.
#[jsonschema::validator(
    schema = r#"{"items":{"pattern":"^(a|a)*\\1b"}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 5_000,
    }
)]
struct UnfinishedCostlyItemsValidator;

// Matching each costly string again in every run hits the backtrack limit once per run.
#[test]
fn test_unfinished_matches_are_not_matched_again_in_later_runs() {
    let instance: serde_json::Value = (0..300)
        .map(|index| serde_json::Value::String(format!("{}!", "a".repeat(30 + index))))
        .collect();
    let errors: Vec<String> = UnfinishedCostlyItemsValidator::iter_errors(&instance)
        .map(|error| error.to_string())
        .collect();
    assert_eq!(
        errors,
        vec!["Error executing regex: Max limit for backtracking count exceeded".to_owned()]
    );
}

#[jsonschema::validator(
    schema = r#"{"properties":{"o":{"patternProperties":{"(?<=ab)c":{"type":"integer"}}}}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedNestedPatternPropertiesValidator;

#[jsonschema::validator(
    schema = r#"{"properties":{"o":{"patternProperties":{"(?<=ab)c":true},"additionalProperties":false}}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedNestedAdditionalPropertiesValidator;

#[jsonschema::validator(
    schema = r#"{"properties":{"o":{"patternProperties":{"(?<=ab)c":true},"unevaluatedProperties":false}}}"#,
    pattern_options = {
        engine = fancy_regex,
        backtrack_limit = 1,
    }
)]
struct UnfinishedNestedUnevaluatedPropertiesValidator;

unfinished_match_location!(
    test_unfinished_match_location_pattern_properties,
    UnfinishedNestedPatternPropertiesValidator,
    r#"{"properties":{"o":{"patternProperties":{"(?<=ab)c":{"type":"integer"}}}}}"#,
    serde_json::json!({"o":{"abc":"x"}})
);
unfinished_match_location!(
    test_unfinished_match_location_additional_properties,
    UnfinishedNestedAdditionalPropertiesValidator,
    r#"{"properties":{"o":{"patternProperties":{"(?<=ab)c":true},"additionalProperties":false}}}"#,
    serde_json::json!({"o":{"abc":1}})
);
unfinished_match_location!(
    test_unfinished_match_location_unevaluated_properties,
    UnfinishedNestedUnevaluatedPropertiesValidator,
    r#"{"properties":{"o":{"patternProperties":{"(?<=ab)c":true},"unevaluatedProperties":false}}}"#,
    serde_json::json!({"o":{"abc":1}})
);

#[jsonschema::validator(
    schema = r#"{"pattern":"^\\d+$"}"#,
    pattern_options = { engine = regex, syntax = engine }
)]
struct EngineSyntaxRegexValidator;

#[jsonschema::validator(
    schema = r#"{"pattern":"^\\d+$"}"#,
    pattern_options = { syntax = engine }
)]
struct EngineSyntaxFancyRegexValidator;

#[jsonschema::validator(
    schema = r#"{"pattern":"^\\d+$"}"#,
    pattern_options = { engine = regex, syntax = ecma262 }
)]
struct EcmaSyntaxRegexValidator;

#[jsonschema::validator(
    schema = r#"{"pattern":"^\\S*$"}"#,
    pattern_options = { engine = regex, syntax = engine }
)]
struct EngineSyntaxNonWhitespaceValidator;

#[jsonschema::validator(
    schema = r#"{"patternProperties":{"^\\d+$":{}},"additionalProperties":false}"#,
    pattern_options = { engine = regex, syntax = engine }
)]
struct EngineSyntaxPatternPropertiesValidator;

#[jsonschema::validator(
    schema = r#"{"patternProperties":{"^\\S*$":false}}"#,
    pattern_options = { engine = regex, syntax = engine }
)]
struct EngineSyntaxPatternPropertiesNonWhitespaceValidator;

#[jsonschema::validator(
    schema = r#"{"format":"regex"}"#,
    validate_formats = true,
    pattern_options = { engine = regex, syntax = engine }
)]
struct EngineSyntaxRegexFormatValidator;

#[jsonschema::validator(
    schema = r#"{"format":"regex"}"#,
    validate_formats = true,
    pattern_options = { syntax = engine }
)]
struct EngineSyntaxFancyRegexFormatValidator;

fn engine_syntax_validator(schema: &serde_json::Value, fancy: bool) -> jsonschema::Validator {
    let options = jsonschema::options().should_validate_formats(true);
    if fancy {
        options.with_pattern_options(
            jsonschema::PatternOptions::fancy_regex().syntax(jsonschema::PatternSyntax::Engine),
        )
    } else {
        options.with_pattern_options(
            jsonschema::PatternOptions::regex().syntax(jsonschema::PatternSyntax::Engine),
        )
    }
    .build(schema)
    .expect("valid schema")
}

#[test_case(serde_json::json!({"pattern": r"^\d+$"}), false, serde_json::json!("٣"), EngineSyntaxRegexValidator::is_valid, true; "regex digit class")]
#[test_case(serde_json::json!({"pattern": r"^\d+$"}), true, serde_json::json!("٣"), EngineSyntaxFancyRegexValidator::is_valid, true; "fancy regex digit class")]
#[test_case(serde_json::json!({"pattern": r"^\S*$"}), false, serde_json::json!("\u{feff}"), EngineSyntaxNonWhitespaceValidator::is_valid, true; "non-whitespace class")]
#[test_case(serde_json::json!({"patternProperties": {r"^\d+$": {}}, "additionalProperties": false}), false, serde_json::json!({"٣": 1}), EngineSyntaxPatternPropertiesValidator::is_valid, true; "patternProperties")]
#[test_case(serde_json::json!({"patternProperties": {r"^\S*$": false}}), false, serde_json::json!({"\u{feff}": 1}), EngineSyntaxPatternPropertiesNonWhitespaceValidator::is_valid, false; "patternProperties non-whitespace class")]
#[test_case(serde_json::json!({"format": "regex"}), false, serde_json::json!("(?P<name>a)"), EngineSyntaxRegexFormatValidator::is_valid, true; "regex format named group")]
#[test_case(serde_json::json!({"format": "regex"}), false, serde_json::json!("(?<=a)b"), EngineSyntaxRegexFormatValidator::is_valid, false; "regex format lookbehind")]
#[test_case(serde_json::json!({"format": "regex"}), true, serde_json::json!("(?<=a)b"), EngineSyntaxFancyRegexFormatValidator::is_valid, true; "fancy regex format lookbehind")]
fn test_engine_syntax_matches_runtime(
    schema: serde_json::Value,
    fancy: bool,
    instance: serde_json::Value,
    is_valid: fn(&serde_json::Value) -> bool,
    expected: bool,
) {
    let runtime = engine_syntax_validator(&schema, fancy);
    assert_eq!(runtime.is_valid(&instance), expected);
    assert_eq!(is_valid(&instance), expected);
}

#[test]
fn test_ecma_syntax_key() {
    assert!(!EcmaSyntaxRegexValidator::is_valid(&serde_json::json!("٣")));
    assert!(EcmaSyntaxRegexValidator::is_valid(&serde_json::json!("3")));
}

// One thread asks both engines about the same string.
#[test]
fn test_engine_syntax_regex_format_is_checked_per_engine() {
    let instance = serde_json::json!("(?<=a)b");
    for _ in 0..2 {
        assert!(EngineSyntaxFancyRegexFormatValidator::is_valid(&instance));
        assert!(!EngineSyntaxRegexFormatValidator::is_valid(&instance));
    }
}

#[jsonschema::validator(
    schema = r##"{"$defs":{"item":{"type":"integer"}},"properties":{"first":{"type":"array","items":{"$ref":"#/$defs/item"}},"second":{"type":"array","items":{"$ref":"#/$defs/item"}},"nested":{"type":"array","items":{"type":"array","items":{"$ref":"#/$defs/item"}}}}}"##
)]
struct ArraysOfSameRefValidator;

// Arrays of one `$ref` report errors through shared code, each at its own schema location.
#[test_case(serde_json::json!({"first": 1, "second": {}, "nested": "x"}) ; "not arrays")]
#[test_case(serde_json::json!({"first": ["a"], "second": [1, "b"], "nested": [[2, "c"], 3]}) ; "invalid items")]
#[test_case(serde_json::json!({"first": [1], "second": [], "nested": [[2]]}) ; "valid")]
fn test_arrays_of_same_ref_match_runtime(instance: serde_json::Value) {
    let schema = serde_json::json!({
        "$defs": {"item": {"type": "integer"}},
        "properties": {
            "first": {"type": "array", "items": {"$ref": "#/$defs/item"}},
            "second": {"type": "array", "items": {"$ref": "#/$defs/item"}},
            "nested": {"type": "array", "items": {"type": "array", "items": {"$ref": "#/$defs/item"}}}
        }
    });
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    assert_validate_parity(
        ArraysOfSameRefValidator::is_valid(&instance),
        ArraysOfSameRefValidator::validate(&instance),
        &runtime,
        &instance,
    );
    assert_iter_errors_parity(
        ArraysOfSameRefValidator::iter_errors(&instance),
        &runtime,
        &instance,
    );
}

#[jsonschema::validator(schema = "true")]
struct EvaluateTrue;

#[jsonschema::validator(schema = "false")]
struct EvaluateFalse;

#[jsonschema::validator(schema = "{}")]
struct EvaluateEmpty;

#[jsonschema::validator(schema = "false", base_uri = "https://example.com/root.json")]
struct EvaluateFalseWithBase;

#[jsonschema::validator(schema = r#"{"type": ["string", "null"]}"#)]
struct EvaluateType;

#[jsonschema::validator(schema = r#"{"maxLength": 3, "minLength": 2, "type": "string"}"#)]
struct EvaluateLengths;

#[jsonschema::validator(
    schema = r#"{"maxLength": 3, "minLength": 2, "type": "string"}"#,
    base_uri = "https://example.com/root.json"
)]
struct EvaluateLengthsWithBase;

#[jsonschema::validator(
    schema = r#"{"maxLength": 3, "minLength": 2, "type": "string"}"#,
    methods = { is_valid = false, validate = false, iter_errors = false, evaluate = true }
)]
struct EvaluateOnly;

#[jsonschema::validator(schema = r#"{"exclusiveMaximum": 5, "minimum": 1, "multipleOf": 2}"#)]
struct EvaluateNumbers;

#[jsonschema::validator(
    schema = r#"{"maximum": 5, "exclusiveMaximum": true, "minimum": 1, "exclusiveMinimum": true}"#,
    draft = Draft4
)]
struct EvaluateDraft4Bounds;

#[jsonschema::validator(schema = r#"{"enum": [1, 2, "a"], "const": 1}"#)]
struct EvaluateValues;

#[jsonschema::validator(schema = r#"{"maxItems": 2, "minItems": 1, "uniqueItems": true}"#)]
struct EvaluateItemCounts;

#[jsonschema::validator(schema = r#"{"uniqueItems": false, "minItems": 1}"#)]
struct EvaluateUniqueItemsFalse;

#[jsonschema::validator(
    schema = r#"{"required": ["a"], "maxProperties": 2, "minProperties": 1, "pattern": "^a"}"#
)]
struct EvaluateObjectsAndPattern;

#[jsonschema::validator(
    schema = r#"{"title": "t", "x-vendor": [1, {"a": null}], "$comment": "c", "minimum": 1}"#
)]
struct EvaluateAnnotations;

#[jsonschema::validator(schema = r#"{"const": 1, "maximum": 1}"#, draft = Draft4)]
struct EvaluateDraft4Const;

#[jsonschema::validator(
    schema = r#"{"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"}"#
)]
struct EvaluateSchemaKeyword;

#[jsonschema::validator(schema = r#"{"format": "email"}"#, draft = Draft202012)]
struct EvaluateFormatAnnotation;

#[jsonschema::validator(schema = r#"{"format": "email"}"#, draft = Draft7)]
struct EvaluateFormatAsserted;

#[jsonschema::validator(schema = r#"{"format": "nope", "minLength": 1}"#, draft = Draft7)]
struct EvaluateUnknownFormat;

#[jsonschema::validator(schema = r#"{"format": "nope"}"#, draft = Draft202012)]
struct EvaluateUnknownFormatAnnotation;

#[jsonschema::validator(schema = r#"{"contentEncoding": "base64", "contentMediaType": "application/json"}"#, draft = Draft7)]
struct EvaluateContentDraft7;

#[jsonschema::validator(schema = r#"{"contentEncoding": "base64"}"#, draft = Draft7)]
struct EvaluateContentEncodingDraft7;

#[jsonschema::validator(schema = r#"{"contentEncoding": "base64", "contentMediaType": "text/x-unknown"}"#, draft = Draft7)]
struct EvaluateContentUnknownMediaTypeDraft7;

#[jsonschema::validator(schema = r#"{"contentMediaType": "text/x-unknown"}"#, draft = Draft7)]
struct EvaluateUnknownMediaTypeDraft7;

#[jsonschema::validator(schema = r#"{"contentEncoding": "rot13"}"#, draft = Draft7)]
struct EvaluateUnknownEncodingDraft7;

#[jsonschema::validator(schema = r#"{"contentEncoding": "rot13", "contentMediaType": "application/json"}"#, draft = Draft7)]
struct EvaluateUnknownEncodingJsonDraft7;

#[jsonschema::validator(schema = r#"{"contentEncoding": "rot13", "contentMediaType": "text/x-unknown"}"#, draft = Draft7)]
struct EvaluateUnknownEncodingAndMediaTypeDraft7;

#[jsonschema::validator(schema = r#"{"contentEncoding": "base64", "contentMediaType": "application/json", "contentSchema": {"type": "integer"}}"#, draft = Draft202012)]
struct EvaluateContent;

#[jsonschema::validator(schema = r#"{"contentSchema": {"type": "integer"}}"#, draft = Draft202012)]
struct EvaluateContentSchemaAlone;

#[jsonschema::validator(
    schema = r#"{"properties": {"a": {"type": "integer"}, "b": false, "c": true, "d": {}, "e~/f": {"minLength": 1, "x-note": 1}}}"#
)]
struct EvaluateProperties;

#[jsonschema::validator(
    schema = r#"{"properties": {"s p": {"type": "string"}, "^a": {"properties": {"b": {"minimum": 2}}}}}"#,
    base_uri = "https://example.com/root.json"
)]
struct EvaluatePropertiesWithBase;

#[jsonschema::validator(
    schema = r#"{"type": "object", "properties": {"b": {"type": "integer"}, "a": {}}, "required": ["a", "b"]}"#
)]
struct EvaluatePropertiesRequired;

#[jsonschema::validator(
    schema = r#"{"patternProperties": {"^a": {"type": "integer"}, "b$": true, "^c": false, "^[0-9]+$": {"minimum": 2}}}"#
)]
struct EvaluatePatternProperties;

#[jsonschema::validator(schema = r#"{"additionalProperties": {"type": "string"}}"#)]
struct EvaluateAdditionalSchema;

#[jsonschema::validator(schema = r#"{"additionalProperties": false}"#)]
struct EvaluateAdditionalFalse;

#[jsonschema::validator(schema = r#"{"additionalProperties": {}}"#)]
struct EvaluateAdditionalEmpty;

#[jsonschema::validator(
    schema = r#"{"properties": {"b": {}, "a": {"type": "integer"}}, "additionalProperties": true}"#
)]
struct EvaluateAdditionalTrue;

#[jsonschema::validator(
    schema = r#"{"properties": {"b": {"type": "integer"}, "a": {}}, "patternProperties": {"^x": {"minLength": 2}}, "additionalProperties": {"type": "string"}}"#
)]
struct EvaluateObjectApplicators;

#[jsonschema::validator(
    schema = r#"{"properties": {"a": {}}, "patternProperties": {"^x": {}}, "additionalProperties": false}"#
)]
struct EvaluateObjectApplicatorsFalse;

#[jsonschema::validator(
    schema = r#"{"properties": {"a": {}}, "required": ["a"], "additionalProperties": false}"#
)]
struct EvaluateClosedRequired;

#[jsonschema::validator(
    schema = r#"{"properties": {"a": {}, "b": {}}, "required": ["a", "b"], "additionalProperties": false}"#
)]
struct EvaluateClosedEveryRequired;

#[jsonschema::validator(
    schema = r#"{"patternProperties": {"^a/b~": {"type": "integer"}, "^ ": {}}, "additionalProperties": {"properties": {"a": false}}}"#,
    base_uri = "https://example.com/root.json"
)]
struct EvaluatePatternsWithBase;

#[jsonschema::validator(schema = r#"{"propertyNames": {"maxLength": 1}}"#)]
struct EvaluatePropertyNames;

#[jsonschema::validator(schema = r#"{"propertyNames": false}"#)]
struct EvaluatePropertyNamesFalse;

#[jsonschema::validator(schema = r#"{"propertyNames": true, "minProperties": 1}"#)]
struct EvaluatePropertyNamesTrue;

#[jsonschema::validator(schema = r#"{"propertyNames": {"format": "email", "minLength": 2}}"#)]
struct EvaluatePropertyNamesFormat;

#[jsonschema::validator(
    schema = r#"{"properties": {"a": {}}, "patternProperties": {"^x": {}}, "additionalProperties": {"type": "integer"}, "propertyNames": {"pattern": "^[a-z]+$", "minLength": 1}}"#
)]
struct EvaluatePropertyNamesBesideApplicators;

#[jsonschema::validator(schema = r#"{"properties": {"a": {"propertyNames": {"maxLength": 1}}}}"#)]
struct EvaluatePropertyNamesNested;

#[jsonschema::validator(schema = r#"{"propertyNames": {"maxLength": 1}, "minProperties": 1}"#, draft = Draft4)]
struct EvaluatePropertyNamesDraft4;

#[jsonschema::validator(schema = r#"{"dependentRequired": {"a": ["b", "c"], "x": ["y"]}}"#, draft = Draft202012)]
struct EvaluateDependentRequired;

#[jsonschema::validator(schema = r#"{"dependentSchemas": {"a": {"required": ["b"]}, "c": true, "d": false, "e": {"x-note": 1, "minProperties": 2}}}"#, draft = Draft202012)]
struct EvaluateDependentSchemas;

#[jsonschema::validator(schema = r#"{"dependencies": {"a": ["b"], "c": {"required": ["d"]}, "e": false, "f": []}}"#, draft = Draft7)]
struct EvaluateDependenciesDraft7;

#[jsonschema::validator(schema = r#"{"dependencies": {"a": ["b"], "c": {"properties": {"c": {"type": "integer"}}}}}"#, draft = Draft4)]
struct EvaluateDependenciesDraft4;

#[jsonschema::validator(schema = r#"{"dependencies": {"a": ["b"], "c": {"minProperties": 3}}}"#, draft = Draft202012)]
struct EvaluateDependencies;

#[jsonschema::validator(schema = r#"{"dependentRequired": {"a": ["b"]}, "minProperties": 1}"#, draft = Draft7)]
struct EvaluateDependentRequiredDraft7;

#[jsonschema::validator(schema = r#"{"prefixItems": [{"type": "integer"}, true], "items": {"type": "string"}}"#, draft = Draft202012)]
struct EvaluatePrefixItems;

#[jsonschema::validator(schema = r#"{"prefixItems": [{}], "items": false}"#, draft = Draft202012)]
struct EvaluateItemsFalse;

#[jsonschema::validator(schema = r#"{"items": {"type": "integer"}}"#, draft = Draft202012)]
struct EvaluateItemsType;

#[jsonschema::validator(schema = r#"{"type": "array", "minItems": 1, "maxItems": 3, "items": {"minimum": 1}}"#, draft = Draft202012)]
struct EvaluateItemsShape;

#[jsonschema::validator(schema = r#"{"items": true, "prefixItems": [{"const": 1}]}"#, draft = Draft202012)]
struct EvaluateItemsTrue;

#[jsonschema::validator(schema = r#"{"items": {"properties": {"a": {"type": "integer"}}}}"#, draft = Draft202012)]
struct EvaluateItemsNested;

#[jsonschema::validator(schema = r#"{"items": [{"type": "integer"}, {}], "additionalItems": {"type": "string"}}"#, draft = Draft7)]
struct EvaluateTupleDraft7;

#[jsonschema::validator(schema = r#"{"items": [{}], "additionalItems": false}"#, draft = Draft7)]
struct EvaluateAdditionalItemsFalse;

#[jsonschema::validator(schema = r#"{"items": [{}, {}], "additionalItems": {"type": "string"}}"#, draft = Draft201909)]
struct EvaluateTupleDraft201909;

#[jsonschema::validator(schema = r#"{"items": {"type": "integer"}, "additionalItems": false}"#, draft = Draft7)]
struct EvaluateAdditionalItemsIgnored;

// Without array-form `items`, the runtime compiles nothing below `additionalItems`, `$id` included.
#[jsonschema::validator(schema = r##"{"additionalItems": {"$id": "http://example.com/tail", "$ref": "#/$defs/s", "$defs": {"s": {"type": "string"}}}}"##, draft = Draft202012)]
struct EvaluateAdditionalItemsWithoutItems;

#[jsonschema::validator(schema = r#"{"items": {"type": "integer"}, "additionalItems": {"$id": "http://a b"}}"#, draft = Draft202012)]
struct EvaluateAdditionalItemsBesideSchemaItems;

#[jsonschema::validator(schema = r#"{"prefixItems": [{"type": "integer"}], "items": {"type": "string"}}"#, draft = Draft201909)]
struct EvaluatePrefixItemsDraft201909;

#[jsonschema::validator(schema = r#"{"items": {"type": "number"}}"#, draft = Draft202012)]
struct EvaluateItemsNumber;

#[jsonschema::validator(schema = r#"{"items": {"type": "array", "items": {"type": "number"}}}"#, draft = Draft202012)]
struct EvaluateNestedItems;

#[jsonschema::validator(schema = r#"{"items": {"type": "array", "minItems": 2, "items": {"type": "number"}}}"#, draft = Draft202012)]
struct EvaluateNestedItemsShape;

#[jsonschema::validator(schema = r#"{"items": {"minimum": 1, "maximum": 5}}"#, draft = Draft202012)]
struct EvaluateItemsRange;

#[jsonschema::validator(schema = r#"{"items": {}}"#, draft = Draft202012)]
struct EvaluateItemsEmpty;

#[jsonschema::validator(schema = r#"{"items": {"type": "string", "title": "t"}}"#, draft = Draft202012)]
struct EvaluateItemsTitled;

#[jsonschema::validator(schema = r#"{"items": {"format": "email"}}"#, draft = Draft202012)]
struct EvaluateItemsFormat;

#[jsonschema::validator(schema = r#"{"items": {"x-unknown": 1, "minimum": 1}}"#, draft = Draft202012)]
struct EvaluateItemsUnknownKeyword;

#[jsonschema::validator(schema = r##"{"$defs": {"n": {"type": "number"}}, "items": {"$ref": "#/$defs/n"}}"##, draft = Draft202012)]
struct EvaluateItemsRefToLeaf;

#[jsonschema::validator(schema = r##"{"$defs": {"list": {"items": {"type": "number"}}}, "$ref": "#/$defs/list"}"##, draft = Draft202012)]
struct EvaluateRefToLeafItems;

#[jsonschema::validator(schema = r##"{"$defs": {"list": {"items": {"type": "number"}}}, "properties": {"a": {"$ref": "#/$defs/list"}, "b": {"$ref": "#/$defs/list"}}}"##, draft = Draft202012)]
struct EvaluateLeafItemsBelowTwoRefs;

#[jsonschema::validator(
    schema = r#"{"items": {"type": "number"}}"#,
    base_uri = "https://example.com/root.json"
)]
struct EvaluateItemsNumberWithBase;

#[jsonschema::validator(schema = r#"{"items": [{"type": "integer"}], "additionalItems": {"minimum": 1}}"#, draft = Draft7)]
struct EvaluateAdditionalItemsLeaf;

#[jsonschema::validator(schema = r#"{"contains": {"type": "integer"}}"#, draft = Draft202012)]
struct EvaluateContains;

#[jsonschema::validator(schema = r#"{"contains": {"type": "integer"}, "minContains": 2, "maxContains": 3}"#, draft = Draft202012)]
struct EvaluateContainsBounds;

#[jsonschema::validator(schema = r#"{"contains": {"type": "integer"}, "minContains": 2}"#, draft = Draft202012)]
struct EvaluateContainsMin;

// Both bounds fail for two matching items, so one unit reports errors from two keywords.
#[jsonschema::validator(schema = r#"{"contains": {"type": "integer"}, "minContains": 3, "maxContains": 1}"#, draft = Draft202012)]
struct EvaluateContainsBoundsCrossed;

#[jsonschema::validator(schema = r#"{"contains": {"type": "integer"}, "maxContains": 1}"#, draft = Draft202012)]
struct EvaluateContainsMax;

#[jsonschema::validator(schema = r#"{"contains": {"const": 1}, "minContains": 0}"#, draft = Draft202012)]
struct EvaluateContainsMinZero;

#[jsonschema::validator(schema = r#"{"contains": false}"#, draft = Draft202012)]
struct EvaluateContainsFalse;

#[jsonschema::validator(schema = r#"{"contains": true}"#, draft = Draft202012)]
struct EvaluateContainsTrue;

#[jsonschema::validator(schema = r#"{"contains": {"properties": {"a": {"type": "integer"}}, "required": ["a"]}}"#, draft = Draft202012)]
struct EvaluateContainsNested;

#[jsonschema::validator(schema = r#"{"contains": {"minimum": 2}, "minContains": 5}"#, draft = Draft7)]
struct EvaluateContainsDraft7;

#[jsonschema::validator(schema = r#"{"contains": {"type": "integer"}, "minItems": 1}"#, draft = Draft4)]
struct EvaluateContainsDraft4;

#[jsonschema::validator(schema = r#"{"allOf": [{"type": "integer"}, {"minimum": 2}, true]}"#, draft = Draft202012)]
struct EvaluateAllOf;

#[jsonschema::validator(schema = r#"{"anyOf": [{"type": "string"}, {"minimum": 2}, {"type": "integer"}]}"#, draft = Draft202012)]
struct EvaluateAnyOf;

#[jsonschema::validator(schema = r#"{"oneOf": [{"type": "integer"}, {"minimum": 2}, {"type": "string"}]}"#, draft = Draft202012)]
struct EvaluateOneOf;

#[jsonschema::validator(schema = r#"{"oneOf": [{"properties": {"a": {"type": "integer"}}, "required": ["a"]}, {"properties": {"b": {"const": 1}}, "required": ["b"]}]}"#, draft = Draft202012)]
struct EvaluateOneOfObjects;

#[jsonschema::validator(schema = r#"{"not": {"type": "integer"}}"#, draft = Draft202012)]
struct EvaluateNot;
#[jsonschema::validator(schema = r#"{"not": {"items": {"type": "string"}, "unevaluatedItems": false}}"#, draft = Draft202012)]
struct EvaluateNotUnwalked;

#[jsonschema::validator(schema = r#"{"if": {"type": "integer"}, "then": {"minimum": 2}, "else": {"type": "string"}}"#, draft = Draft202012)]
struct EvaluateConditional;

#[jsonschema::validator(schema = r#"{"if": {"type": "integer"}, "then": {"minimum": 2}}"#, draft = Draft7)]
struct EvaluateIfThen;

#[jsonschema::validator(schema = r#"{"if": {"type": "integer"}, "else": {"type": "string"}}"#, draft = Draft202012)]
struct EvaluateIfElse;

#[jsonschema::validator(schema = r#"{"if": {"type": "integer"}, "minLength": 1}"#, draft = Draft202012)]
struct EvaluateIfAlone;

// The runtime compiles `if`, `then` and `else` only together, leaving these references unresolved.
#[jsonschema::validator(schema = r##"{"if": {"$ref": "#/$defs/missing"}, "minLength": 1}"##, draft = Draft202012)]
struct EvaluateIfWithoutBranches;

#[jsonschema::validator(schema = r##"{"then": {"$ref": "#/$defs/missing"}, "else": {"$ref": "#/$defs/missing"}, "minLength": 1}"##, draft = Draft202012)]
struct EvaluateBranchesWithoutIf;

#[jsonschema::validator(schema = r#"{"anyOf": [{"allOf": [{"type": "array"}, {"items": {"not": {"type": "string"}}}]}, {"oneOf": [{"type": "object"}, {"type": "null"}]}]}"#, draft = Draft202012)]
struct EvaluateNestedCombinators;
#[jsonschema::validator(schema = r##"{"$defs": {"a": {"type": "integer", "minimum": 2}}, "$ref": "#/$defs/a"}"##, draft = Draft202012)]
struct EvaluateRef;
#[jsonschema::validator(schema = r##"{"$defs": {"a": {"type": "integer"}}, "$ref": "#/$defs/a", "minimum": 5, "title": "t"}"##, draft = Draft202012)]
struct EvaluateRefSiblings;
#[jsonschema::validator(schema = r##"{"definitions": {"a": {"type": "integer"}}, "$ref": "#/definitions/a", "minimum": 5, "title": "t"}"##, draft = Draft7)]
struct EvaluateRefSiblingsDraft7;
#[jsonschema::validator(schema = r##"{"properties": {"foo": {"$ref": "#/$defs/a"}}, "$defs": {"a": {"minLength": 2, "title": "t"}}}"##, draft = Draft202012)]
struct EvaluateRefBelowProperties;
#[jsonschema::validator(schema = r##"{"type": "object", "properties": {"next": {"$ref": "#"}, "v": {"type": "integer"}}}"##, draft = Draft202012)]
struct EvaluateRefRecursive;
#[jsonschema::validator(schema = r##"{"anyOf": [{"$ref": "#/$defs/a"}, {"type": "null"}], "$defs": {"a": {"type": "integer"}}}"##, draft = Draft202012)]
struct EvaluateRefInBranches;
#[jsonschema::validator(schema = r##"{"$ref": "#/$defs/f", "$defs": {"f": false}}"##, draft = Draft202012)]
struct EvaluateRefFalse;
#[jsonschema::validator(schema = r##"{"$defs": {"a": {"$ref": "#/$defs/b"}, "b": {"type": "integer"}}, "$ref": "#/$defs/a"}"##, draft = Draft202012)]
struct EvaluateRefChain;
#[jsonschema::validator(schema = r##"{"$id": "https://example.com/root", "$defs": {"a": {"$anchor": "here", "type": "integer"}}, "$ref": "#here"}"##, draft = Draft202012)]
struct EvaluateRefAnchor;
#[jsonschema::validator(schema = r##"{"allOf": [{"$ref": "#/$defs/a"}, {"$ref": "#/$defs/a"}], "$defs": {"a": {"type": "integer"}}}"##, draft = Draft202012)]
struct EvaluateRefTwice;
#[jsonschema::validator(schema = r##"{"$ref": "#", "type": "integer"}"##, draft = Draft202012)]
struct EvaluateRefRoot;
#[jsonschema::validator(schema = r##"{"$defs": {"a": {"$ref": "#/$defs/a", "type": "integer"}}, "$ref": "#/$defs/a"}"##, draft = Draft202012)]
struct EvaluateRefSelf;
#[jsonschema::validator(schema = r##"{"properties": {"a": {"type": "integer"}, "b": {"$ref": "#/properties/a"}}}"##, draft = Draft202012)]
struct EvaluateRefToProperty;
#[jsonschema::validator(schema = r##"{"items": {"$ref": "#/$defs/a"}, "$defs": {"a": {"type": "integer"}}}"##, draft = Draft202012)]
struct EvaluateRefBelowItems;
#[jsonschema::validator(schema = r##"{"definitions": {"a": {"type": "integer"}}, "properties": {"a": {"$ref": "#/definitions/a"}}}"##, draft = Draft4)]
struct EvaluateRefDraft4;
#[jsonschema::validator(schema = r##"{"contains": {"$ref": "#/$defs/a"}, "$defs": {"a": {"type": "integer"}}}"##, draft = Draft202012)]
struct EvaluateRefBelowContains;
#[jsonschema::validator(schema = r##"{"definitions": {"a": {"type": "integer"}, "b": {"$id": "http://example.com/b"}}, "$ref": "#/definitions/a", "properties": {"x": {"$ref": "http://example.com/b"}}}"##, draft = Draft7)]
struct EvaluateRefIgnoredSiblings;
#[jsonschema::validator(schema = r##"{"$defs": {"s": {"propertyNames": {"$ref": "#/$defs/s"}, "maxLength": 3}}, "$ref": "#/$defs/s"}"##, draft = Draft202012)]
struct EvaluateRefBelowNames;
#[jsonschema::validator(schema = r##"{"definitions": {"node": {"oneOf": [{"required": ["a"], "properties": {"a": {"const": 1}, "c": {"$ref": "#/definitions/node"}}}, {"properties": {"b": {"const": 1}, "c": {"$ref": "#/definitions/node"}}}, {"type": "string"}]}}, "$ref": "#/definitions/node"}"##, draft = Draft7)]
struct EvaluateRefCycleInBranches;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/tree", "$dynamicAnchor": "node", "type": "object", "properties": {"a": {"$dynamicRef": "#node"}, "b": {"$dynamicRef": "#node"}}}"##, draft = Draft202012)]
struct EvaluateTwoDynamicRefsToOneAnchor;
#[jsonschema::validator(schema = r##"{"$id": "http://example.com/root", "items": {"$id": "x", "items": false, "prefixItems": [{"type": "integer"}]}, "properties": {"a": {"$ref": "#/items/prefixItems/0"}}}"##, draft = Draft202012)]
struct EvaluateRefByPointerPastBooleanItems;
#[jsonschema::validator(schema = r##"{"contentEncoding": "base64", "contentMediaType": "application/json"}"##, draft = Draft6)]
struct EvaluateContentDraft6;
#[jsonschema::validator(schema = r#"{"contentEncoding": "base64"}"#, draft = Draft6)]
struct EvaluateContentEncodingDraft6;
#[jsonschema::validator(schema = r#"{"contentMediaType": "application/json"}"#, draft = Draft6)]
struct EvaluateContentMediaTypeDraft6;
#[jsonschema::validator(schema = r##"{"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minimum": 1, "allOf": [{"$ref": "#/$defs/node"}]}}}"##, draft = Draft202012)]
struct EvaluateRefCycleOnOneInstance;
#[jsonschema::validator(schema = r##"{"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"maximum": 1, "allOf": [{"$ref": "#/$defs/node"}]}}}"##, draft = Draft202012)]
struct EvaluateFailingRefCycleOnOneInstance;
#[jsonschema::validator(schema = r##"{"minimum": 1, "allOf": [{"$ref": "#"}]}"##, draft = Draft202012)]
struct EvaluateRefCycleThroughRoot;
#[jsonschema::validator(schema = r##"{"properties": {"b": {"maximum": 1, "allOf": [{"$ref": "#/properties/b"}]}}}"##, draft = Draft202012)]
struct EvaluateRefCycleThroughInlineTarget;
#[jsonschema::validator(schema = r##"{"properties": {"b": {"$ref": "#/$defs/a"}}, "$defs": {"a": {"minimum": 1, "allOf": [{"$ref": "#/$defs/b"}]}, "b": {"maximum": 1, "allOf": [{"$ref": "#/$defs/a"}]}}}"##, draft = Draft202012)]
struct EvaluateMutualRefCycleOnOneInstance;
#[jsonschema::validator(schema = r##"{"$ref": "#/$defs/node", "$defs": {"node": {"required": ["a"], "properties": {"c": {"$ref": "#/$defs/node"}}, "allOf": [{"$ref": "#/$defs/node"}]}}}"##, draft = Draft202012)]
struct EvaluateRefCycleOnOneInstanceAndBelow;
#[jsonschema::validator(schema = r##"{"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"maximum": 1, "anyOf": [{"$ref": "#/$defs/node"}, {"type": "string"}]}}}"##, draft = Draft202012)]
struct EvaluateRefCycleThroughAnyOf;
#[jsonschema::validator(schema = r##"{"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"maximum": 1, "oneOf": [{"$ref": "#/$defs/node"}, {"type": "string"}]}}}"##, draft = Draft202012)]
struct EvaluateRefCycleThroughOneOf;
#[jsonschema::validator(schema = r##"{"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"maximum": 1, "if": {"$ref": "#/$defs/node"}, "then": {"title": "t"}, "else": {"title": "e"}}}}"##, draft = Draft202012)]
struct EvaluateRefCycleThroughIf;
#[jsonschema::validator(schema = r##"{"maximum": 1, "anyOf": [{"$ref": "#"}, {"type": "string"}]}"##, draft = Draft202012)]
struct EvaluateRefCycleThroughRootAndAnyOf;
#[jsonschema::validator(schema = r##"{"$ref": "#/$defs/node", "$defs": {"node": {"properties": {"c": {"$ref": "#/$defs/node"}}, "anyOf": [{"$ref": "#/$defs/node"}, {"required": ["x"]}], "minProperties": 2}}}"##, draft = Draft202012)]
struct EvaluateRefCycleThroughAnyOfBelow;

#[jsonschema::validator(schema = r##"{"properties": {"a": {}}, "unevaluatedProperties": false}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesFalse;
#[jsonschema::validator(schema = r##"{"properties": {"a": {}}, "unevaluatedProperties": {"type": "integer"}}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesSchema;
#[jsonschema::validator(schema = r##"{"allOf": [{"properties": {"a": true}}], "unevaluatedProperties": false}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesAllOf;
#[jsonschema::validator(schema = r##"{"anyOf": [{"properties": {"a": {"type": "string"}}}, {"properties": {"b": true}}], "unevaluatedProperties": false}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesFailingBranch;
#[jsonschema::validator(schema = r##"{"$defs": {"x": {"properties": {"a": true}}}, "$ref": "#/$defs/x", "unevaluatedProperties": false}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesRef;
#[jsonschema::validator(schema = r##"{"if": {"properties": {"a": {"const": 1}}}, "then": {"properties": {"b": true}}, "else": {"properties": {"c": true}}, "unevaluatedProperties": false}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesConditional;
#[jsonschema::validator(schema = r##"{"patternProperties": {"^x": true}, "unevaluatedProperties": {"minimum": 2}}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesPatterns;
#[jsonschema::validator(schema = r##"{"dependentSchemas": {"a": {"properties": {"b": true}}}, "unevaluatedProperties": false}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesDependentSchemas;
#[jsonschema::validator(schema = r##"{"properties": {"o": {"properties": {"p": true}, "unevaluatedProperties": false}}}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesNested;
#[jsonschema::validator(schema = r##"{"prefixItems": [{"type": "integer"}], "unevaluatedItems": false}"##, draft = Draft202012)]
struct EvaluateUnevaluatedItemsFalse;
#[jsonschema::validator(schema = r##"{"prefixItems": [true], "unevaluatedItems": {"type": "integer"}}"##, draft = Draft202012)]
struct EvaluateUnevaluatedItemsSchema;
#[jsonschema::validator(schema = r##"{"contains": {"type": "string"}, "unevaluatedItems": {"type": "integer"}}"##, draft = Draft202012)]
struct EvaluateUnevaluatedItemsContains;
#[jsonschema::validator(schema = r##"{"anyOf": [{"prefixItems": [{"type": "string"}]}, {"prefixItems": [true, true]}], "unevaluatedItems": false}"##, draft = Draft202012)]
struct EvaluateUnevaluatedItemsFailingBranch;
#[jsonschema::validator(schema = r##"{"items": [{"type": "integer"}], "additionalItems": {"type": "string"}, "unevaluatedItems": false}"##, draft = Draft201909)]
struct EvaluateUnevaluatedItemsDraft201909;
#[jsonschema::validator(schema = r##"{"items": [true], "unevaluatedItems": {"type": "integer"}}"##, draft = Draft201909)]
struct EvaluateUnevaluatedItemsTupleDraft201909;
#[jsonschema::validator(schema = r##"{"allOf": [{"properties": {"a": {"type": "string"}}}], "unevaluatedProperties": {"type": "integer"}}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesFailingAllOf;
#[jsonschema::validator(schema = r##"{"$defs": {"x": {"properties": {"b": {"type": "string"}}}}, "$ref": "#/$defs/x", "unevaluatedProperties": {"type": "integer"}}"##, draft = Draft202012)]
struct EvaluateUnevaluatedPropertiesFailingRef;
#[jsonschema::validator(schema = r##"{"allOf": [{"prefixItems": [{"type": "string"}]}], "unevaluatedItems": {"type": "integer"}}"##, draft = Draft202012)]
struct EvaluateUnevaluatedItemsFailingAllOf;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "list", "$defs": {"foo": {"$dynamicAnchor": "items", "type": "string"}, "list": {"$id": "list", "type": "array", "items": {"$dynamicRef": "#items"}, "$defs": {"items": {"$dynamicAnchor": "items"}}}}}"##, draft = Draft202012)]
struct EvaluateDynamicRefOutermostAnchor;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/plain", "$defs": {"foo": {"type": "string"}}, "items": {"$dynamicRef": "#/$defs/foo"}}"##, draft = Draft202012)]
struct EvaluateDynamicRefWithoutAnchor;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/anchored", "$ref": "list", "$defs": {"foo": {"$dynamicAnchor": "items", "type": "string"}, "list": {"$id": "list", "type": "array", "items": {"$dynamicRef": "#items"}, "$defs": {"items": {"$anchor": "items", "type": "integer"}}}}}"##, draft = Draft202012)]
struct EvaluateDynamicRefToPlainAnchor;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "properties": {"foo": {"$recursiveRef": "#"}}, "additionalProperties": false}"##, draft = Draft201909)]
struct EvaluateRecursiveRefWithoutAnchor;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/recursive", "$recursiveAnchor": true, "properties": {"marker": {"type": "boolean"}, "t": {"$ref": "tree"}}, "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "properties": {"child": {"$recursiveRef": "#"}}}}}"##, draft = Draft201909)]
struct EvaluateRecursiveRefOutermostAnchor;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/unanchored", "properties": {"marker": {"type": "boolean"}, "t": {"$ref": "tree"}}, "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "properties": {"child": {"$recursiveRef": "#"}, "marker": {"type": "integer"}}}}}"##, draft = Draft201909)]
struct EvaluateRecursiveRefUnanchoredOuter;
#[jsonschema::validator(schema = r##"{"oneOf": [{"properties": {"kind": {"const": "a"}, "v": {"type": "integer"}}, "required": ["kind"]}, {"properties": {"kind": {"const": "b"}, "v": {"type": "string"}}, "required": ["kind"]}, {"properties": {"kind": {"enum": ["c", "d"]}}, "required": ["kind"], "minProperties": 3}]}"##, draft = Draft202012)]
struct EvaluateDiscriminatedOneOf;
#[jsonschema::validator(schema = r##"{"anyOf": [{"properties": {"kind": {"const": "a"}, "v": {"type": "integer"}}, "required": ["kind"]}, {"properties": {"kind": {"const": "b"}, "v": {"type": "string"}}, "required": ["kind"]}, {"properties": {"kind": {"enum": ["c", "d"]}}, "required": ["kind"], "minProperties": 3}]}"##, draft = Draft202012)]
struct EvaluateDiscriminatedAnyOf;
#[jsonschema::validator(schema = r##"{"$id": "https://example.com/root", "properties": {"a": {"$ref": "inner"}}, "$defs": {"inner": {"$id": "inner", "not": {"$ref": "#/$defs/small"}, "$defs": {"small": {"maximum": 2}}}}}"##, draft = Draft202012)]
struct EvaluateNotAroundRefInEmbeddedResource;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/branches", "$ref": "list", "$defs": {"foo": {"$dynamicAnchor": "items", "type": "string"}, "list": {"$id": "list", "type": "array", "items": {"anyOf": [{"$dynamicRef": "#items"}, {"type": "null"}]}, "$defs": {"items": {"$dynamicAnchor": "items"}}}}}"##, draft = Draft202012)]
struct EvaluateDynamicRefInAnyOf;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/condition", "$recursiveAnchor": true, "properties": {"marker": {"type": "boolean"}, "t": {"$ref": "tree"}}, "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "if": {"properties": {"child": {"$recursiveRef": "#"}}}, "then": {"required": ["ok"]}}}}"##, draft = Draft201909)]
struct EvaluateRecursiveRefInIf;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/derived", "$ref": "base", "$defs": {"derived": {"$dynamicAnchor": "addons", "properties": {"bar": {"type": "string"}}}, "base": {"$id": "base", "unevaluatedProperties": false, "type": "object", "properties": {"foo": {"type": "string"}}, "$dynamicRef": "#addons", "$defs": {"defaultAddons": {"$dynamicAnchor": "addons"}}}}}"##, draft = Draft202012)]
struct EvaluateDynamicRefUnevaluatedProperties;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/items", "$recursiveAnchor": true, "type": "object", "properties": {"l": {"$ref": "list"}}, "$defs": {"list": {"$id": "list", "$recursiveAnchor": true, "type": ["array", "integer"], "unevaluatedItems": {"$recursiveRef": "#"}}}}"##, draft = Draft201909)]
struct EvaluateRecursiveRefUnevaluatedItems;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/hops", "properties": {"c": {"$ref": "c"}}, "$defs": {"c": {"$id": "c", "$recursiveAnchor": true, "type": "object", "properties": {"marker": {"type": "boolean"}}, "anyOf": [{"properties": {"d": {"$ref": "d"}}}, {"required": ["x"]}]}, "d": {"$id": "d", "$recursiveAnchor": true, "properties": {"next": {"$recursiveRef": "#"}}}}}"##, draft = Draft201909)]
struct EvaluateRecursiveRefThroughBranchReference;
#[jsonschema::validator(schema = r##"{"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/nesting", "$defs": {"myobject": {"$id": "myobject.json", "$recursiveAnchor": true, "anyOf": [{"type": "string"}, {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}]}}, "anyOf": [{"type": "integer"}, {"$ref": "#/$defs/myobject"}]}"##, draft = Draft201909)]
struct EvaluateRecursiveRefInAnchoredBranch;
#[jsonschema::validator(
    schema = r##"{"$id": "http://localhost:1234/scope_change_defs1.json", "type": "object", "properties": {"list": {"$ref": "baseUriChangeFolder/"}}, "$defs": {"baz": {"$id": "baseUriChangeFolder/", "type": "array", "items": {"$ref": "folderInteger.json"}}}}"##,
    resources = { "http://localhost:1234/baseUriChangeFolder/folderInteger.json" => { schema = r##"{"type": "integer"}"## } },
    draft = Draft202012
)]
struct EvaluateFolderId;
#[jsonschema::validator(schema = r#"{"type":"integer","even":true,"minimum":2,"title":"t"}"#, keywords = { "even" => crate::even_factory })]
struct EvaluateCustomKeyword;
#[jsonschema::validator(schema = r#"{"properties":{"a":{"multi":true}},"multi":true}"#, keywords = { "multi" => crate::multi_error_factory })]
struct EvaluateCustomKeywordErrors;
#[jsonschema::validator(schema = r#"{"description":"strings-bad","title":"t"}"#, keywords = { "description" => crate::string_gate_factory })]
struct EvaluateCustomKeywordKnownName;
#[jsonschema::validator(schema = r#"{"type":"string","format":"currency"}"#, draft = Draft7, formats = { "currency" => crate::is_currency_format })]
struct EvaluateCustomFormat;
#[jsonschema::validator(schema = r#"{"format":"even","items":{"format":"even"}}"#, draft = Draft7, formats = { "even" => number(crate::is_even_number) })]
struct EvaluateNumberFormat;
#[jsonschema::validator(schema = r#"{"type":"integer","format":"even"}"#, draft = Draft7, formats = { "even" => number(crate::is_even_number) })]
struct EvaluateIntegerNumberFormat;
#[jsonschema::validator(schema = r#"{"type":"string","format":"currency"}"#, draft = Draft201909, validate_formats = true, formats = { "currency" => crate::is_currency_format })]
struct EvaluateCustomFormatAsserted;
#[jsonschema::validator(schema = r#"{"type":"string","format":"email"}"#, draft = Draft7, validate_formats = true, formats = { "email" => crate::is_literal_x })]
struct EvaluateCustomFormatOverride;
#[jsonschema::validator(schema = r#"{"type":"string","format":"made-up"}"#, draft = Draft7, validate_formats = true, ignore_unknown_formats = true)]
struct EvaluateIgnoredUnknownFormat;
#[jsonschema::validator(schema = r#"{"contentMediaType":"text/hex"}"#, draft = Draft7, content_media_types = { "text/hex" => crate::is_hex_content })]
struct EvaluateCustomContentMediaType;
#[jsonschema::validator(schema = r#"{"contentEncoding":"prefixed","contentMediaType":"application/json"}"#, draft = Draft7, content_encodings = { "prefixed" => { check = crate::check_prefixed, convert = crate::convert_prefixed } })]
struct EvaluateCustomContentEncoding;
#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-validation","properties":{"a":{"minimum":10,"type":"string"},"b":false},"minLength":2,"required":["c"]}"#,
    draft = Draft202012,
    resources = { "json-schema:///meta/no-validation" => { schema = r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true}}"# } }
)]
struct EvaluateNoValidationVocabulary;
#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-applicator","type":["array","object"],"items":{"type":"string"},"contains":{"type":"integer"},"properties":{"a":false}}"#,
    draft = Draft202012,
    resources = { "json-schema:///meta/no-applicator" => { schema = r#"{"$id":"json-schema:///meta/no-applicator","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true}}"# } }
)]
struct EvaluateNoApplicatorVocabulary;
#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/optional","type":"number","minimum":2}"#,
    draft = Draft201909,
    resources = { "json-schema:///meta/optional" => { schema = r#"{"$id":"json-schema:///meta/optional","$schema":"https://json-schema.org/draft/2019-09/schema","$vocabulary":{"https://json-schema.org/draft/2019-09/vocab/core":true,"https://json-schema.org/draft/2019-09/vocab/validation":true,"json-schema:///vocab/custom":false}}"# } }
)]
struct EvaluateOptionalUnknownVocabulary;
#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-validation-2019","properties":{"a":{"minimum":10}},"unevaluatedProperties":{"type":"string"}}"#,
    draft = Draft201909,
    resources = { "json-schema:///meta/no-validation-2019" => { schema = r#"{"$id":"json-schema:///meta/no-validation-2019","$schema":"https://json-schema.org/draft/2019-09/schema","$vocabulary":{"https://json-schema.org/draft/2019-09/vocab/core":true,"https://json-schema.org/draft/2019-09/vocab/applicator":true}}"# } }
)]
struct EvaluateNoValidationVocabularyDraft201909;
#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-unevaluated","properties":{"a":{"type":"string"}},"unevaluatedProperties":false,"unevaluatedItems":false}"#,
    draft = Draft202012,
    resources = { "json-schema:///meta/no-unevaluated" => { schema = r#"{"$id":"json-schema:///meta/no-unevaluated","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true}}"# } }
)]
struct EvaluateNoUnevaluatedVocabulary;
#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/no-content","type":"string","contentEncoding":"base64","contentMediaType":"application/json","contentSchema":{"type":"integer"}}"#,
    draft = Draft202012,
    resources = { "json-schema:///meta/no-content" => { schema = r#"{"$id":"json-schema:///meta/no-content","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true}}"# } }
)]
struct EvaluateNoContentVocabulary;
#[jsonschema::validator(
    schema = r##"{"$schema":"json-schema:///meta/no-validation","properties":{"a":{"$ref":"#/$defs/x"}},"$defs":{"x":{"minimum":10,"properties":{"b":false}}}}"##,
    draft = Draft202012,
    resources = { "json-schema:///meta/no-validation" => { schema = r##"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true}}"## } }
)]
struct EvaluateNoValidationVocabularyThroughRef;
#[jsonschema::validator(
    schema = r#"{"$ref":"http://example.com/ignore-prefix-items.json","type":"array"}"#,
    draft = Draft202012,
    resources = { "http://example.com/ignore-prefix-items.json" => { schema = r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"http://example.com/ignore-prefix-items.json","prefixItems":[{"type":"integer"}]}"# } }
)]
struct EvaluateRefToEarlierDraft;
#[jsonschema::validator(
    schema = r#"{"allOf":[{"properties":{"foo":true}},{"$ref":"http://example.com/dependent-required.json"}],"type":"object"}"#,
    draft = Draft7,
    resources = { "http://example.com/dependent-required.json" => { schema = r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"http://example.com/dependent-required.json","dependentRequired":{"foo":["bar"]}}"# } }
)]
struct EvaluateRefToLaterDraft;
#[jsonschema::validator(
    schema = r##"{"$ref":"http://example.com/draft7.json"}"##,
    draft = Draft202012,
    resources = { "http://example.com/draft7.json" => { schema = r##"{"$schema":"http://json-schema.org/draft-07/schema#","$id":"http://example.com/draft7.json","definitions":{"s":{"type":"string"}},"properties":{"a":{"$ref":"#/definitions/s","minLength":3}}}"## } }
)]
struct EvaluateRefToDraft7Siblings;
#[jsonschema::validator(schema = r##"{"$ref":"http://example.com/a","$defs":{"a":{"$id":"http://example.com/a","$schema":"http://json-schema.org/draft-07/schema#","definitions":{"s":{"type":"string"}},"properties":{"a":{"$ref":"#/definitions/s","minLength":3}}}}}"##, draft = Draft202012)]
struct EvaluateRefToEmbeddedEarlierDraft;
const OTHER: &str = r##"{"$defs": {"s": {"type": "string"}}, "type": "object", "properties": {"n": {"$ref": "#/$defs/s"}}}"##;
#[jsonschema::validator(
    schema = r##"{"properties": {"a": {"$ref": "http://example.com/other.json"}, "b": {"$ref": "http://example.com/other.json#/$defs/s"}}}"##,
    resources = { "http://example.com/other.json" => { schema = r##"{"$defs": {"s": {"type": "string"}}, "type": "object", "properties": {"n": {"$ref": "#/$defs/s"}}}"## } },
    draft = Draft202012
)]
struct EvaluateExternalRef;
#[jsonschema::validator(schema = r##"{"$id": "http://example.com/root.json", "properties": {"foo": {"$id": "foo.json", "type": "object", "properties": {"bar": {"$ref": "#/$defs/s"}}, "$defs": {"s": {"minimum": 3}}}}}"##, draft = Draft202012)]
struct EvaluateEmbeddedId;
#[jsonschema::validator(schema = r##"{"properties": {"a": {"$id": "http://example.com/a.json", "type": "integer", "minimum": 2}}}"##, draft = Draft202012)]
struct EvaluateEmbeddedIdWithoutRootId;
#[jsonschema::validator(schema = r##"{"$defs": {"x": {"$id": "http://example.com/x", "type": "string"}}, "properties": {"a": {"$ref": "http://example.com/x"}}}"##, draft = Draft202012)]
struct EvaluateRefToEmbeddedResource;
#[jsonschema::validator(schema = r##"{"$id": "http://example.com/root", "properties": {"a": {"$ref": "#/$defs/x/properties/y"}}, "$defs": {"x": {"$id": "x", "properties": {"y": {"type": "integer"}}}}}"##, draft = Draft202012)]
struct EvaluateRefByPointerIntoEmbeddedResource;
#[jsonschema::validator(schema = r##"{"properties": {"a": {"id": "http://example.com/a.json", "type": "object", "properties": {"b": {"$ref": "#/definitions/s"}}, "definitions": {"s": {"minimum": 3}}}}}"##, draft = Draft4)]
struct EvaluateEmbeddedIdDraft4;
#[jsonschema::validator(schema = r##"{"$id": "urn:uuid:deadbeef-1234-ffff-ffff-4321feebdaed", "properties": {"a": {"$ref": "urn:uuid:deadbeef-1234-ffff-ffff-4321feebdaed#/$defs/s"}}, "$defs": {"s": {"type": "string"}}}"##, draft = Draft202012)]
struct EvaluateUrnBase;
#[jsonschema::validator(
    schema = r##"{"$id": "http://example.com/root", "properties": {"a": {"$ref": "json-schema:///other"}, "b": {"$ref": "json-schema:///other#/$defs/s"}, "c": {"$ref": "json-schema:///never"}, "d": {"$id": "json-schema:///inner", "maxLength": 0}}, "propertyNames": {"$ref": "json-schema:///names"}}"##,
    resources = {
        "json-schema:///other" => { schema = r#"{"maxLength": 0, "$defs": {"s": {"minimum": 1}}}"# },
        "json-schema:///never" => { schema = "false" },
        "json-schema:///names" => { schema = r#"{"maxLength": 1}"# },
    },
    draft = Draft202012
)]
struct EvaluateResourcesUnderDefaultBase;

fn evaluation_outputs(evaluation: &jsonschema::Evaluation) -> [serde_json::Value; 6] {
    let annotations = evaluation
        .iter_annotations()
        .map(|entry| {
            serde_json::json!([
                entry.schema_location,
                entry.absolute_keyword_location.map(jsonschema::Uri::as_str),
                entry.instance_location.as_str(),
                entry.annotations.value()
            ])
        })
        .collect();
    let errors = evaluation
        .iter_errors()
        .map(|entry| {
            serde_json::json!([
                entry.schema_location,
                entry.absolute_keyword_location.map(jsonschema::Uri::as_str),
                entry.instance_location.as_str(),
                entry.error.to_string()
            ])
        })
        .collect();
    [
        serde_json::to_value(evaluation.list()).expect("list output serializes"),
        serde_json::to_value(evaluation.hierarchical()).expect("hierarchical output serializes"),
        serde_json::Value::Bool(evaluation.flag().valid),
        serde_json::Value::Array(annotations),
        serde_json::Value::Array(errors),
        output_units(evaluation.root()),
    ]
}

// Every unit below `unit`, read through `OutputUnit` accessors.
fn output_units(unit: jsonschema::OutputUnit<'_>) -> serde_json::Value {
    serde_json::json!({
        "valid": unit.valid(),
        "evaluationPath": unit.evaluation_path().as_str(),
        "schemaLocation": unit.schema_location(),
        "instanceLocation": unit.instance_location().as_str(),
        "annotations": unit.annotations().map(jsonschema::output::Annotations::value),
        "droppedAnnotations": unit.dropped_annotations().map(jsonschema::output::Annotations::value),
        "errors": unit
            .errors()
            .iter()
            .map(|error| [error.keyword(), error.message()])
            .collect::<Vec<_>>(),
        "details": unit.children().map(output_units).collect::<Vec<_>>(),
    })
}

type EvaluateFn = fn(&serde_json::Value) -> jsonschema::Evaluation;

#[test_case(EvaluateTrue::evaluate, serde_json::json!(true), None, Draft::Draft202012 ; "true_schema")]
#[test_case(EvaluateFalse::evaluate, serde_json::json!(false), None, Draft::Draft202012 ; "false_schema")]
#[test_case(EvaluateEmpty::evaluate, serde_json::json!({}), None, Draft::Draft202012 ; "empty_schema")]
#[test_case(EvaluateFalseWithBase::evaluate, serde_json::json!(false), Some("https://example.com/root.json"), Draft::Draft202012 ; "false_schema_with_base")]
#[test_case(EvaluateType::evaluate, serde_json::json!({"type": ["string", "null"]}), None, Draft::Draft202012 ; "type")]
#[test_case(EvaluateLengths::evaluate, serde_json::json!({"maxLength": 3, "minLength": 2, "type": "string"}), None, Draft::Draft202012 ; "lengths")]
#[test_case(EvaluateLengthsWithBase::evaluate, serde_json::json!({"maxLength": 3, "minLength": 2, "type": "string"}), Some("https://example.com/root.json"), Draft::Draft202012 ; "lengths_with_base")]
#[test_case(EvaluateOnly::evaluate, serde_json::json!({"maxLength": 3, "minLength": 2, "type": "string"}), None, Draft::Draft202012 ; "evaluate_only")]
#[test_case(EvaluateNumbers::evaluate, serde_json::json!({"exclusiveMaximum": 5, "minimum": 1, "multipleOf": 2}), None, Draft::Draft202012 ; "numbers")]
#[test_case(EvaluateDraft4Bounds::evaluate, serde_json::json!({"maximum": 5, "exclusiveMaximum": true, "minimum": 1, "exclusiveMinimum": true}), None, Draft::Draft4 ; "draft4_bounds")]
#[test_case(EvaluateValues::evaluate, serde_json::json!({"enum": [1, 2, "a"], "const": 1}), None, Draft::Draft202012 ; "values")]
#[test_case(EvaluateItemCounts::evaluate, serde_json::json!({"maxItems": 2, "minItems": 1, "uniqueItems": true}), None, Draft::Draft202012 ; "item_counts")]
#[test_case(EvaluateUniqueItemsFalse::evaluate, serde_json::json!({"uniqueItems": false, "minItems": 1}), None, Draft::Draft202012 ; "unique_items_false")]
#[test_case(EvaluateAnnotations::evaluate, serde_json::json!({"title": "t", "x-vendor": [1, {"a": null}], "$comment": "c", "minimum": 1}), None, Draft::Draft202012 ; "annotations")]
#[test_case(EvaluateDraft4Const::evaluate, serde_json::json!({"const": 1, "maximum": 1}), None, Draft::Draft4 ; "draft4_const_annotation")]
#[test_case(EvaluateSchemaKeyword::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"}), None, Draft::Draft202012 ; "schema_keyword")]
#[test_case(EvaluateFormatAnnotation::evaluate, serde_json::json!({"format": "email"}), None, Draft::Draft202012 ; "format_annotation")]
#[test_case(EvaluateFormatAsserted::evaluate, serde_json::json!({"format": "email"}), None, Draft::Draft7 ; "format_asserted")]
#[test_case(EvaluateUnknownFormat::evaluate, serde_json::json!({"format": "nope", "minLength": 1}), None, Draft::Draft7 ; "unknown_format_asserted")]
#[test_case(EvaluateUnknownFormatAnnotation::evaluate, serde_json::json!({"format": "nope"}), None, Draft::Draft202012 ; "unknown_format_annotation")]
#[test_case(EvaluateContentDraft7::evaluate, serde_json::json!({"contentEncoding": "base64", "contentMediaType": "application/json"}), None, Draft::Draft7 ; "content_draft7")]
#[test_case(EvaluateContentEncodingDraft7::evaluate, serde_json::json!({"contentEncoding": "base64"}), None, Draft::Draft7 ; "content_encoding_draft7")]
#[test_case(EvaluateContentUnknownMediaTypeDraft7::evaluate, serde_json::json!({"contentEncoding": "base64", "contentMediaType": "text/x-unknown"}), None, Draft::Draft7 ; "content_unknown_media_type_draft7")]
#[test_case(EvaluateUnknownMediaTypeDraft7::evaluate, serde_json::json!({"contentMediaType": "text/x-unknown"}), None, Draft::Draft7 ; "unknown_media_type_draft7")]
#[test_case(EvaluateUnknownEncodingDraft7::evaluate, serde_json::json!({"contentEncoding": "rot13"}), None, Draft::Draft7 ; "unknown_encoding_draft7")]
#[test_case(EvaluateUnknownEncodingJsonDraft7::evaluate, serde_json::json!({"contentEncoding": "rot13", "contentMediaType": "application/json"}), None, Draft::Draft7 ; "unknown_encoding_json_draft7")]
#[test_case(EvaluateUnknownEncodingAndMediaTypeDraft7::evaluate, serde_json::json!({"contentEncoding": "rot13", "contentMediaType": "text/x-unknown"}), None, Draft::Draft7 ; "unknown_encoding_and_media_type_draft7")]
#[test_case(EvaluateContent::evaluate, serde_json::json!({"contentEncoding": "base64", "contentMediaType": "application/json", "contentSchema": {"type": "integer"}}), None, Draft::Draft202012 ; "content_annotations")]
#[test_case(EvaluateContentSchemaAlone::evaluate, serde_json::json!({"contentSchema": {"type": "integer"}}), None, Draft::Draft202012 ; "content_schema_alone")]
#[test_case(EvaluateObjectsAndPattern::evaluate, serde_json::json!({"required": ["a"], "maxProperties": 2, "minProperties": 1, "pattern": "^a"}), None, Draft::Draft202012 ; "objects_and_pattern")]
#[test_case(EvaluateProperties::evaluate, serde_json::json!({"properties": {"a": {"type": "integer"}, "b": false, "c": true, "d": {}, "e~/f": {"minLength": 1, "x-note": 1}}}), None, Draft::Draft202012 ; "properties")]
#[test_case(EvaluatePropertiesWithBase::evaluate, serde_json::json!({"properties": {"s p": {"type": "string"}, "^a": {"properties": {"b": {"minimum": 2}}}}}), Some("https://example.com/root.json"), Draft::Draft202012 ; "properties_with_base")]
#[test_case(EvaluatePropertiesRequired::evaluate, serde_json::json!({"type": "object", "properties": {"b": {"type": "integer"}, "a": {}}, "required": ["a", "b"]}), None, Draft::Draft202012 ; "properties_required")]
#[test_case(EvaluatePatternProperties::evaluate, serde_json::json!({"patternProperties": {"^a": {"type": "integer"}, "b$": true, "^c": false, "^[0-9]+$": {"minimum": 2}}}), None, Draft::Draft202012 ; "pattern_properties")]
#[test_case(EvaluateAdditionalSchema::evaluate, serde_json::json!({"additionalProperties": {"type": "string"}}), None, Draft::Draft202012 ; "additional_schema")]
#[test_case(EvaluateAdditionalFalse::evaluate, serde_json::json!({"additionalProperties": false}), None, Draft::Draft202012 ; "additional_false")]
#[test_case(EvaluateAdditionalEmpty::evaluate, serde_json::json!({"additionalProperties": {}}), None, Draft::Draft202012 ; "additional_empty")]
#[test_case(EvaluateAdditionalTrue::evaluate, serde_json::json!({"properties": {"b": {}, "a": {"type": "integer"}}, "additionalProperties": true}), None, Draft::Draft202012 ; "additional_true")]
#[test_case(EvaluateObjectApplicators::evaluate, serde_json::json!({"properties": {"b": {"type": "integer"}, "a": {}}, "patternProperties": {"^x": {"minLength": 2}}, "additionalProperties": {"type": "string"}}), None, Draft::Draft202012 ; "object_applicators")]
#[test_case(EvaluateObjectApplicatorsFalse::evaluate, serde_json::json!({"properties": {"a": {}}, "patternProperties": {"^x": {}}, "additionalProperties": false}), None, Draft::Draft202012 ; "object_applicators_false")]
#[test_case(EvaluateClosedRequired::evaluate, serde_json::json!({"properties": {"a": {}}, "required": ["a"], "additionalProperties": false}), None, Draft::Draft202012 ; "closed_required")]
#[test_case(EvaluateClosedEveryRequired::evaluate, serde_json::json!({"properties": {"a": {}, "b": {}}, "required": ["a", "b"], "additionalProperties": false}), None, Draft::Draft202012 ; "closed_every_required")]
#[test_case(EvaluatePatternsWithBase::evaluate, serde_json::json!({"patternProperties": {"^a/b~": {"type": "integer"}, "^ ": {}}, "additionalProperties": {"properties": {"a": false}}}), Some("https://example.com/root.json"), Draft::Draft202012 ; "patterns_with_base")]
#[test_case(EvaluatePropertyNames::evaluate, serde_json::json!({"propertyNames": {"maxLength": 1}}), None, Draft::Draft202012 ; "property_names")]
#[test_case(EvaluatePropertyNamesFalse::evaluate, serde_json::json!({"propertyNames": false}), None, Draft::Draft202012 ; "property_names_false")]
#[test_case(EvaluatePropertyNamesTrue::evaluate, serde_json::json!({"propertyNames": true, "minProperties": 1}), None, Draft::Draft202012 ; "property_names_true")]
#[test_case(EvaluatePropertyNamesFormat::evaluate, serde_json::json!({"propertyNames": {"format": "email", "minLength": 2}}), None, Draft::Draft202012 ; "property_names_format")]
#[test_case(EvaluatePropertyNamesBesideApplicators::evaluate, serde_json::json!({"properties": {"a": {}}, "patternProperties": {"^x": {}}, "additionalProperties": {"type": "integer"}, "propertyNames": {"pattern": "^[a-z]+$", "minLength": 1}}), None, Draft::Draft202012 ; "property_names_beside_applicators")]
#[test_case(EvaluatePropertyNamesNested::evaluate, serde_json::json!({"properties": {"a": {"propertyNames": {"maxLength": 1}}}}), None, Draft::Draft202012 ; "property_names_nested")]
#[test_case(EvaluatePropertyNamesDraft4::evaluate, serde_json::json!({"propertyNames": {"maxLength": 1}, "minProperties": 1}), None, Draft::Draft4 ; "property_names_draft4")]
#[test_case(EvaluateDependentRequired::evaluate, serde_json::json!({"dependentRequired": {"a": ["b", "c"], "x": ["y"]}}), None, Draft::Draft202012 ; "dependent_required")]
#[test_case(EvaluateDependentSchemas::evaluate, serde_json::json!({"dependentSchemas": {"a": {"required": ["b"]}, "c": true, "d": false, "e": {"x-note": 1, "minProperties": 2}}}), None, Draft::Draft202012 ; "dependent_schemas")]
#[test_case(EvaluateDependenciesDraft7::evaluate, serde_json::json!({"dependencies": {"a": ["b"], "c": {"required": ["d"]}, "e": false, "f": []}}), None, Draft::Draft7 ; "dependencies_draft7")]
#[test_case(EvaluateDependenciesDraft4::evaluate, serde_json::json!({"dependencies": {"a": ["b"], "c": {"properties": {"c": {"type": "integer"}}}}}), None, Draft::Draft4 ; "dependencies_draft4")]
#[test_case(EvaluateDependencies::evaluate, serde_json::json!({"dependencies": {"a": ["b"], "c": {"minProperties": 3}}}), None, Draft::Draft202012 ; "dependencies_draft2020")]
#[test_case(EvaluateDependentRequiredDraft7::evaluate, serde_json::json!({"dependentRequired": {"a": ["b"]}, "minProperties": 1}), None, Draft::Draft7 ; "dependent_required_draft7")]
#[test_case(EvaluatePrefixItems::evaluate, serde_json::json!({"prefixItems": [{"type": "integer"}, true], "items": {"type": "string"}}), None, Draft::Draft202012 ; "prefix_items")]
#[test_case(EvaluateItemsFalse::evaluate, serde_json::json!({"prefixItems": [{}], "items": false}), None, Draft::Draft202012 ; "items_false")]
#[test_case(EvaluateItemsType::evaluate, serde_json::json!({"items": {"type": "integer"}}), None, Draft::Draft202012 ; "items_type")]
#[test_case(EvaluateItemsShape::evaluate, serde_json::json!({"type": "array", "minItems": 1, "maxItems": 3, "items": {"minimum": 1}}), None, Draft::Draft202012 ; "items_shape")]
#[test_case(EvaluateItemsTrue::evaluate, serde_json::json!({"items": true, "prefixItems": [{"const": 1}]}), None, Draft::Draft202012 ; "items_true")]
#[test_case(EvaluateItemsNested::evaluate, serde_json::json!({"items": {"properties": {"a": {"type": "integer"}}}}), None, Draft::Draft202012 ; "items_nested")]
#[test_case(EvaluateTupleDraft7::evaluate, serde_json::json!({"items": [{"type": "integer"}, {}], "additionalItems": {"type": "string"}}), None, Draft::Draft7 ; "tuple_draft7")]
#[test_case(EvaluateAdditionalItemsFalse::evaluate, serde_json::json!({"items": [{}], "additionalItems": false}), None, Draft::Draft7 ; "additional_items_false")]
#[test_case(EvaluateTupleDraft201909::evaluate, serde_json::json!({"items": [{}, {}], "additionalItems": {"type": "string"}}), None, Draft::Draft201909 ; "tuple_draft2019")]
#[test_case(EvaluateAdditionalItemsIgnored::evaluate, serde_json::json!({"items": {"type": "integer"}, "additionalItems": false}), None, Draft::Draft7 ; "additional_items_ignored")]
#[test_case(EvaluateAdditionalItemsWithoutItems::evaluate, serde_json::json!({"additionalItems": {"$id": "http://example.com/tail", "$ref": "#/$defs/s", "$defs": {"s": {"type": "string"}}}}), None, Draft::Draft202012 ; "additional_items_without_items")]
#[test_case(EvaluateAdditionalItemsBesideSchemaItems::evaluate, serde_json::json!({"items": {"type": "integer"}, "additionalItems": {"$id": "http://a b"}}), None, Draft::Draft202012 ; "additional_items_beside_schema_items")]
#[test_case(EvaluatePrefixItemsDraft201909::evaluate, serde_json::json!({"prefixItems": [{"type": "integer"}], "items": {"type": "string"}}), None, Draft::Draft201909 ; "prefix_items_draft2019")]
#[test_case(EvaluateItemsNumber::evaluate, serde_json::json!({"items": {"type": "number"}}), None, Draft::Draft202012 ; "items_number")]
#[test_case(EvaluateNestedItems::evaluate, serde_json::json!({"items": {"type": "array", "items": {"type": "number"}}}), None, Draft::Draft202012 ; "nested_items")]
#[test_case(EvaluateNestedItemsShape::evaluate, serde_json::json!({"items": {"type": "array", "minItems": 2, "items": {"type": "number"}}}), None, Draft::Draft202012 ; "nested_items_shape")]
#[test_case(EvaluateItemsRange::evaluate, serde_json::json!({"items": {"minimum": 1, "maximum": 5}}), None, Draft::Draft202012 ; "items_range")]
#[test_case(EvaluateItemsEmpty::evaluate, serde_json::json!({"items": {}}), None, Draft::Draft202012 ; "items_empty")]
#[test_case(EvaluateItemsTitled::evaluate, serde_json::json!({"items": {"type": "string", "title": "t"}}), None, Draft::Draft202012 ; "items_titled")]
#[test_case(EvaluateItemsFormat::evaluate, serde_json::json!({"items": {"format": "email"}}), None, Draft::Draft202012 ; "items_format")]
#[test_case(EvaluateItemsUnknownKeyword::evaluate, serde_json::json!({"items": {"x-unknown": 1, "minimum": 1}}), None, Draft::Draft202012 ; "items_unknown_keyword")]
#[test_case(EvaluateItemsRefToLeaf::evaluate, serde_json::json!({"$defs": {"n": {"type": "number"}}, "items": {"$ref": "#/$defs/n"}}), None, Draft::Draft202012 ; "items_ref_to_leaf")]
#[test_case(EvaluateRefToLeafItems::evaluate, serde_json::json!({"$defs": {"list": {"items": {"type": "number"}}}, "$ref": "#/$defs/list"}), None, Draft::Draft202012 ; "ref_to_leaf_items")]
#[test_case(EvaluateLeafItemsBelowTwoRefs::evaluate, serde_json::json!({"$defs": {"list": {"items": {"type": "number"}}}, "properties": {"a": {"$ref": "#/$defs/list"}, "b": {"$ref": "#/$defs/list"}}}), None, Draft::Draft202012 ; "leaf_items_below_two_refs")]
#[test_case(EvaluateItemsNumberWithBase::evaluate, serde_json::json!({"items": {"type": "number"}}), Some("https://example.com/root.json"), Draft::Draft202012 ; "items_number_with_base")]
#[test_case(EvaluateAdditionalItemsLeaf::evaluate, serde_json::json!({"items": [{"type": "integer"}], "additionalItems": {"minimum": 1}}), None, Draft::Draft7 ; "additional_items_leaf")]
#[test_case(EvaluateContains::evaluate, serde_json::json!({"contains": {"type": "integer"}}), None, Draft::Draft202012 ; "contains")]
#[test_case(EvaluateContainsBounds::evaluate, serde_json::json!({"contains": {"type": "integer"}, "minContains": 2, "maxContains": 3}), None, Draft::Draft202012 ; "contains_bounds")]
#[test_case(EvaluateContainsBoundsCrossed::evaluate, serde_json::json!({"contains": {"type": "integer"}, "minContains": 3, "maxContains": 1}), None, Draft::Draft202012 ; "contains_bounds_crossed")]
#[test_case(EvaluateContainsMin::evaluate, serde_json::json!({"contains": {"type": "integer"}, "minContains": 2}), None, Draft::Draft202012 ; "contains_min")]
#[test_case(EvaluateContainsMax::evaluate, serde_json::json!({"contains": {"type": "integer"}, "maxContains": 1}), None, Draft::Draft202012 ; "contains_max")]
#[test_case(EvaluateContainsMinZero::evaluate, serde_json::json!({"contains": {"const": 1}, "minContains": 0}), None, Draft::Draft202012 ; "contains_min_zero")]
#[test_case(EvaluateContainsFalse::evaluate, serde_json::json!({"contains": false}), None, Draft::Draft202012 ; "contains_false")]
#[test_case(EvaluateContainsTrue::evaluate, serde_json::json!({"contains": true}), None, Draft::Draft202012 ; "contains_true")]
#[test_case(EvaluateContainsNested::evaluate, serde_json::json!({"contains": {"properties": {"a": {"type": "integer"}}, "required": ["a"]}}), None, Draft::Draft202012 ; "contains_nested")]
#[test_case(EvaluateContainsDraft7::evaluate, serde_json::json!({"contains": {"minimum": 2}, "minContains": 5}), None, Draft::Draft7 ; "contains_draft7")]
#[test_case(EvaluateContainsDraft4::evaluate, serde_json::json!({"contains": {"type": "integer"}, "minItems": 1}), None, Draft::Draft4 ; "contains_draft4")]
#[test_case(EvaluateAllOf::evaluate, serde_json::json!({"allOf": [{"type": "integer"}, {"minimum": 2}, true]}), None, Draft::Draft202012 ; "all_of")]
#[test_case(EvaluateAnyOf::evaluate, serde_json::json!({"anyOf": [{"type": "string"}, {"minimum": 2}, {"type": "integer"}]}), None, Draft::Draft202012 ; "any_of")]
#[test_case(EvaluateOneOf::evaluate, serde_json::json!({"oneOf": [{"type": "integer"}, {"minimum": 2}, {"type": "string"}]}), None, Draft::Draft202012 ; "one_of")]
#[test_case(EvaluateOneOfObjects::evaluate, serde_json::json!({"oneOf": [{"properties": {"a": {"type": "integer"}}, "required": ["a"]}, {"properties": {"b": {"const": 1}}, "required": ["b"]}]}), None, Draft::Draft202012 ; "one_of_objects")]
#[test_case(EvaluateNot::evaluate, serde_json::json!({"not": {"type": "integer"}}), None, Draft::Draft202012 ; "not")]
#[test_case(EvaluateNotUnwalked::evaluate, serde_json::json!({"not": {"items": {"type": "string"}, "unevaluatedItems": false}}), None, Draft::Draft202012 ; "not unwalked")]
#[test_case(EvaluateConditional::evaluate, serde_json::json!({"if": {"type": "integer"}, "then": {"minimum": 2}, "else": {"type": "string"}}), None, Draft::Draft202012 ; "conditional")]
#[test_case(EvaluateIfThen::evaluate, serde_json::json!({"if": {"type": "integer"}, "then": {"minimum": 2}}), None, Draft::Draft7 ; "if_then")]
#[test_case(EvaluateIfElse::evaluate, serde_json::json!({"if": {"type": "integer"}, "else": {"type": "string"}}), None, Draft::Draft202012 ; "if_else")]
#[test_case(EvaluateIfAlone::evaluate, serde_json::json!({"if": {"type": "integer"}, "minLength": 1}), None, Draft::Draft202012 ; "if_alone")]
#[test_case(EvaluateIfWithoutBranches::evaluate, serde_json::json!({"if": {"$ref": "#/$defs/missing"}, "minLength": 1}), None, Draft::Draft202012 ; "if_without_branches")]
#[test_case(EvaluateBranchesWithoutIf::evaluate, serde_json::json!({"then": {"$ref": "#/$defs/missing"}, "else": {"$ref": "#/$defs/missing"}, "minLength": 1}), None, Draft::Draft202012 ; "branches_without_if")]
#[test_case(EvaluateNestedCombinators::evaluate, serde_json::json!({"anyOf": [{"allOf": [{"type": "array"}, {"items": {"not": {"type": "string"}}}]}, {"oneOf": [{"type": "object"}, {"type": "null"}]}]}), None, Draft::Draft202012 ; "nested_combinators")]
#[test_case(EvaluateRef::evaluate, serde_json::json!({"$defs": {"a": {"type": "integer", "minimum": 2}}, "$ref": "#/$defs/a"}), None, Draft::Draft202012 ; "ref")]
#[test_case(EvaluateRefSiblings::evaluate, serde_json::json!({"$defs": {"a": {"type": "integer"}}, "$ref": "#/$defs/a", "minimum": 5, "title": "t"}), None, Draft::Draft202012 ; "ref_siblings")]
#[test_case(EvaluateRefSiblingsDraft7::evaluate, serde_json::json!({"definitions": {"a": {"type": "integer"}}, "$ref": "#/definitions/a", "minimum": 5, "title": "t"}), None, Draft::Draft7 ; "ref_siblings_draft7")]
#[test_case(EvaluateRefBelowProperties::evaluate, serde_json::json!({"properties": {"foo": {"$ref": "#/$defs/a"}}, "$defs": {"a": {"minLength": 2, "title": "t"}}}), None, Draft::Draft202012 ; "ref_below_properties")]
#[test_case(EvaluateRefRecursive::evaluate, serde_json::json!({"type": "object", "properties": {"next": {"$ref": "#"}, "v": {"type": "integer"}}}), None, Draft::Draft202012 ; "ref_recursive")]
#[test_case(EvaluateRefInBranches::evaluate, serde_json::json!({"anyOf": [{"$ref": "#/$defs/a"}, {"type": "null"}], "$defs": {"a": {"type": "integer"}}}), None, Draft::Draft202012 ; "ref_in_branches")]
#[test_case(EvaluateRefFalse::evaluate, serde_json::json!({"$ref": "#/$defs/f", "$defs": {"f": false}}), None, Draft::Draft202012 ; "ref_false")]
#[test_case(EvaluateRefChain::evaluate, serde_json::json!({"$defs": {"a": {"$ref": "#/$defs/b"}, "b": {"type": "integer"}}, "$ref": "#/$defs/a"}), None, Draft::Draft202012 ; "ref_chain")]
#[test_case(EvaluateRefAnchor::evaluate, serde_json::json!({"$id": "https://example.com/root", "$defs": {"a": {"$anchor": "here", "type": "integer"}}, "$ref": "#here"}), None, Draft::Draft202012 ; "ref_anchor")]
#[test_case(EvaluateRefTwice::evaluate, serde_json::json!({"allOf": [{"$ref": "#/$defs/a"}, {"$ref": "#/$defs/a"}], "$defs": {"a": {"type": "integer"}}}), None, Draft::Draft202012 ; "ref_twice")]
#[test_case(EvaluateRefRoot::evaluate, serde_json::json!({"$ref": "#", "type": "integer"}), None, Draft::Draft202012 ; "ref_root")]
#[test_case(EvaluateRefSelf::evaluate, serde_json::json!({"$defs": {"a": {"$ref": "#/$defs/a", "type": "integer"}}, "$ref": "#/$defs/a"}), None, Draft::Draft202012 ; "ref_self")]
#[test_case(EvaluateRefToProperty::evaluate, serde_json::json!({"properties": {"a": {"type": "integer"}, "b": {"$ref": "#/properties/a"}}}), None, Draft::Draft202012 ; "ref_to_property")]
#[test_case(EvaluateRefBelowItems::evaluate, serde_json::json!({"items": {"$ref": "#/$defs/a"}, "$defs": {"a": {"type": "integer"}}}), None, Draft::Draft202012 ; "ref_below_items")]
#[test_case(EvaluateRefDraft4::evaluate, serde_json::json!({"definitions": {"a": {"type": "integer"}}, "properties": {"a": {"$ref": "#/definitions/a"}}}), None, Draft::Draft4 ; "ref_draft4")]
#[test_case(EvaluateRefBelowContains::evaluate, serde_json::json!({"contains": {"$ref": "#/$defs/a"}, "$defs": {"a": {"type": "integer"}}}), None, Draft::Draft202012 ; "ref_below_contains")]
#[test_case(EvaluateRefIgnoredSiblings::evaluate, serde_json::json!({"definitions": {"a": {"type": "integer"}, "b": {"$id": "http://example.com/b"}}, "$ref": "#/definitions/a", "properties": {"x": {"$ref": "http://example.com/b"}}}), None, Draft::Draft7 ; "ref_ignored_siblings")]
#[test_case(EvaluateRefBelowNames::evaluate, serde_json::json!({"$defs": {"s": {"propertyNames": {"$ref": "#/$defs/s"}, "maxLength": 3}}, "$ref": "#/$defs/s"}), None, Draft::Draft202012 ; "ref_below_names")]
#[test_case(EvaluateUnevaluatedPropertiesFalse::evaluate, serde_json::json!({"properties": {"a": {}}, "unevaluatedProperties": false}), None, Draft::Draft202012 ; "unevaluated_properties_false")]
#[test_case(EvaluateUnevaluatedPropertiesSchema::evaluate, serde_json::json!({"properties": {"a": {}}, "unevaluatedProperties": {"type": "integer"}}), None, Draft::Draft202012 ; "unevaluated_properties_schema")]
#[test_case(EvaluateUnevaluatedPropertiesAllOf::evaluate, serde_json::json!({"allOf": [{"properties": {"a": true}}], "unevaluatedProperties": false}), None, Draft::Draft202012 ; "unevaluated_properties_all_of")]
#[test_case(EvaluateUnevaluatedPropertiesFailingBranch::evaluate, serde_json::json!({"anyOf": [{"properties": {"a": {"type": "string"}}}, {"properties": {"b": true}}], "unevaluatedProperties": false}), None, Draft::Draft202012 ; "unevaluated_properties_failing_branch")]
#[test_case(EvaluateUnevaluatedPropertiesRef::evaluate, serde_json::json!({"$defs": {"x": {"properties": {"a": true}}}, "$ref": "#/$defs/x", "unevaluatedProperties": false}), None, Draft::Draft202012 ; "unevaluated_properties_ref")]
#[test_case(EvaluateUnevaluatedPropertiesConditional::evaluate, serde_json::json!({"if": {"properties": {"a": {"const": 1}}}, "then": {"properties": {"b": true}}, "else": {"properties": {"c": true}}, "unevaluatedProperties": false}), None, Draft::Draft202012 ; "unevaluated_properties_conditional")]
#[test_case(EvaluateUnevaluatedPropertiesPatterns::evaluate, serde_json::json!({"patternProperties": {"^x": true}, "unevaluatedProperties": {"minimum": 2}}), None, Draft::Draft202012 ; "unevaluated_properties_patterns")]
#[test_case(EvaluateUnevaluatedPropertiesDependentSchemas::evaluate, serde_json::json!({"dependentSchemas": {"a": {"properties": {"b": true}}}, "unevaluatedProperties": false}), None, Draft::Draft202012 ; "unevaluated_properties_dependent_schemas")]
#[test_case(EvaluateUnevaluatedPropertiesNested::evaluate, serde_json::json!({"properties": {"o": {"properties": {"p": true}, "unevaluatedProperties": false}}}), None, Draft::Draft202012 ; "unevaluated_properties_nested")]
#[test_case(EvaluateUnevaluatedItemsFalse::evaluate, serde_json::json!({"prefixItems": [{"type": "integer"}], "unevaluatedItems": false}), None, Draft::Draft202012 ; "unevaluated_items_false")]
#[test_case(EvaluateUnevaluatedItemsSchema::evaluate, serde_json::json!({"prefixItems": [true], "unevaluatedItems": {"type": "integer"}}), None, Draft::Draft202012 ; "unevaluated_items_schema")]
#[test_case(EvaluateUnevaluatedItemsContains::evaluate, serde_json::json!({"contains": {"type": "string"}, "unevaluatedItems": {"type": "integer"}}), None, Draft::Draft202012 ; "unevaluated_items_contains")]
#[test_case(EvaluateUnevaluatedItemsFailingBranch::evaluate, serde_json::json!({"anyOf": [{"prefixItems": [{"type": "string"}]}, {"prefixItems": [true, true]}], "unevaluatedItems": false}), None, Draft::Draft202012 ; "unevaluated_items_failing_branch")]
#[test_case(EvaluateUnevaluatedItemsDraft201909::evaluate, serde_json::json!({"items": [{"type": "integer"}], "additionalItems": {"type": "string"}, "unevaluatedItems": false}), None, Draft::Draft201909 ; "unevaluated_items_draft2019")]
#[test_case(EvaluateUnevaluatedItemsTupleDraft201909::evaluate, serde_json::json!({"items": [true], "unevaluatedItems": {"type": "integer"}}), None, Draft::Draft201909 ; "unevaluated_items_tuple_draft2019")]
#[test_case(EvaluateUnevaluatedPropertiesFailingAllOf::evaluate, serde_json::json!({"allOf": [{"properties": {"a": {"type": "string"}}}], "unevaluatedProperties": {"type": "integer"}}), None, Draft::Draft202012 ; "unevaluated_properties_failing_all_of")]
#[test_case(EvaluateUnevaluatedPropertiesFailingRef::evaluate, serde_json::json!({"$defs": {"x": {"properties": {"b": {"type": "string"}}}}, "$ref": "#/$defs/x", "unevaluatedProperties": {"type": "integer"}}), None, Draft::Draft202012 ; "unevaluated_properties_failing_ref")]
#[test_case(EvaluateUnevaluatedItemsFailingAllOf::evaluate, serde_json::json!({"allOf": [{"prefixItems": [{"type": "string"}]}], "unevaluatedItems": {"type": "integer"}}), None, Draft::Draft202012 ; "unevaluated_items_failing_all_of")]
#[test_case(EvaluateDynamicRefOutermostAnchor::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "list", "$defs": {"foo": {"$dynamicAnchor": "items", "type": "string"}, "list": {"$id": "list", "type": "array", "items": {"$dynamicRef": "#items"}, "$defs": {"items": {"$dynamicAnchor": "items"}}}}}), None, Draft::Draft202012 ; "dynamic_ref_outermost_anchor")]
#[test_case(EvaluateDynamicRefWithoutAnchor::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/plain", "$defs": {"foo": {"type": "string"}}, "items": {"$dynamicRef": "#/$defs/foo"}}), None, Draft::Draft202012 ; "dynamic_ref_without_anchor")]
#[test_case(EvaluateDynamicRefToPlainAnchor::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/anchored", "$ref": "list", "$defs": {"foo": {"$dynamicAnchor": "items", "type": "string"}, "list": {"$id": "list", "type": "array", "items": {"$dynamicRef": "#items"}, "$defs": {"items": {"$anchor": "items", "type": "integer"}}}}}), None, Draft::Draft202012 ; "dynamic_ref_to_plain_anchor")]
#[test_case(EvaluateRecursiveRefWithoutAnchor::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "properties": {"foo": {"$recursiveRef": "#"}}, "additionalProperties": false}), None, Draft::Draft201909 ; "recursive_ref_without_anchor")]
#[test_case(EvaluateRecursiveRefOutermostAnchor::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/recursive", "$recursiveAnchor": true, "properties": {"marker": {"type": "boolean"}, "t": {"$ref": "tree"}}, "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "properties": {"child": {"$recursiveRef": "#"}}}}}), None, Draft::Draft201909 ; "recursive_ref_outermost_anchor")]
#[test_case(EvaluateRecursiveRefUnanchoredOuter::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/unanchored", "properties": {"marker": {"type": "boolean"}, "t": {"$ref": "tree"}}, "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "properties": {"child": {"$recursiveRef": "#"}, "marker": {"type": "integer"}}}}}), None, Draft::Draft201909 ; "recursive_ref_unanchored_outer")]
#[test_case(EvaluateDynamicRefInAnyOf::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/branches", "$ref": "list", "$defs": {"foo": {"$dynamicAnchor": "items", "type": "string"}, "list": {"$id": "list", "type": "array", "items": {"anyOf": [{"$dynamicRef": "#items"}, {"type": "null"}]}, "$defs": {"items": {"$dynamicAnchor": "items"}}}}}), None, Draft::Draft202012 ; "dynamic_ref_in_any_of")]
#[test_case(EvaluateRecursiveRefInIf::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/condition", "$recursiveAnchor": true, "properties": {"marker": {"type": "boolean"}, "t": {"$ref": "tree"}}, "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "if": {"properties": {"child": {"$recursiveRef": "#"}}}, "then": {"required": ["ok"]}}}}), None, Draft::Draft201909 ; "recursive_ref_in_if")]
#[test_case(EvaluateDynamicRefUnevaluatedProperties::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/derived", "$ref": "base", "$defs": {"derived": {"$dynamicAnchor": "addons", "properties": {"bar": {"type": "string"}}}, "base": {"$id": "base", "unevaluatedProperties": false, "type": "object", "properties": {"foo": {"type": "string"}}, "$dynamicRef": "#addons", "$defs": {"defaultAddons": {"$dynamicAnchor": "addons"}}}}}), None, Draft::Draft202012 ; "dynamic_ref_unevaluated_properties")]
#[test_case(EvaluateRecursiveRefUnevaluatedItems::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/items", "$recursiveAnchor": true, "type": "object", "properties": {"l": {"$ref": "list"}}, "$defs": {"list": {"$id": "list", "$recursiveAnchor": true, "type": ["array", "integer"], "unevaluatedItems": {"$recursiveRef": "#"}}}}), None, Draft::Draft201909 ; "recursive_ref_unevaluated_items")]
#[test_case(EvaluateRecursiveRefThroughBranchReference::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/hops", "properties": {"c": {"$ref": "c"}}, "$defs": {"c": {"$id": "c", "$recursiveAnchor": true, "type": "object", "properties": {"marker": {"type": "boolean"}}, "anyOf": [{"properties": {"d": {"$ref": "d"}}}, {"required": ["x"]}]}, "d": {"$id": "d", "$recursiveAnchor": true, "properties": {"next": {"$recursiveRef": "#"}}}}}), None, Draft::Draft201909 ; "recursive_ref_through_branch_reference")]
#[test_case(EvaluateRecursiveRefInAnchoredBranch::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/nesting", "$defs": {"myobject": {"$id": "myobject.json", "$recursiveAnchor": true, "anyOf": [{"type": "string"}, {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}]}}, "anyOf": [{"type": "integer"}, {"$ref": "#/$defs/myobject"}]}), None, Draft::Draft201909 ; "recursive_ref_in_anchored_branch")]
#[test_case(EvaluateRefToEmbeddedEarlierDraft::evaluate, serde_json::json!({"$ref": "http://example.com/a", "$defs": {"a": {"$id": "http://example.com/a", "$schema": "http://json-schema.org/draft-07/schema#", "definitions": {"s": {"type": "string"}}, "properties": {"a": {"$ref": "#/definitions/s", "minLength": 3}}}}}), None, Draft::Draft202012 ; "ref_to_embedded_earlier_draft")]
#[test_case(EvaluateDiscriminatedOneOf::evaluate, serde_json::json!({"oneOf": [{"properties": {"kind": {"const": "a"}, "v": {"type": "integer"}}, "required": ["kind"]}, {"properties": {"kind": {"const": "b"}, "v": {"type": "string"}}, "required": ["kind"]}, {"properties": {"kind": {"enum": ["c", "d"]}}, "required": ["kind"], "minProperties": 3}]}), None, Draft::Draft202012 ; "discriminated_one_of")]
#[test_case(EvaluateDiscriminatedAnyOf::evaluate, serde_json::json!({"anyOf": [{"properties": {"kind": {"const": "a"}, "v": {"type": "integer"}}, "required": ["kind"]}, {"properties": {"kind": {"const": "b"}, "v": {"type": "string"}}, "required": ["kind"]}, {"properties": {"kind": {"enum": ["c", "d"]}}, "required": ["kind"], "minProperties": 3}]}), None, Draft::Draft202012 ; "discriminated_any_of")]
#[test_case(EvaluateRefCycleInBranches::evaluate, serde_json::json!({"definitions": {"node": {"oneOf": [{"required": ["a"], "properties": {"a": {"const": 1}, "c": {"$ref": "#/definitions/node"}}}, {"properties": {"b": {"const": 1}, "c": {"$ref": "#/definitions/node"}}}, {"type": "string"}]}}, "$ref": "#/definitions/node"}), None, Draft::Draft7 ; "ref_cycle_in_branches")]
#[test_case(EvaluateNotAroundRefInEmbeddedResource::evaluate, serde_json::json!({"$id": "https://example.com/root", "properties": {"a": {"$ref": "inner"}}, "$defs": {"inner": {"$id": "inner", "not": {"$ref": "#/$defs/small"}, "$defs": {"small": {"maximum": 2}}}}}), None, Draft::Draft202012 ; "not_around_ref_in_embedded_resource")]
#[test_case(EvaluateTwoDynamicRefsToOneAnchor::evaluate, serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/tree", "$dynamicAnchor": "node", "type": "object", "properties": {"a": {"$dynamicRef": "#node"}, "b": {"$dynamicRef": "#node"}}}), None, Draft::Draft202012 ; "two_dynamic_refs_to_one_anchor")]
#[test_case(EvaluateRefByPointerPastBooleanItems::evaluate, serde_json::json!({"$id": "http://example.com/root", "items": {"$id": "x", "items": false, "prefixItems": [{"type": "integer"}]}, "properties": {"a": {"$ref": "#/items/prefixItems/0"}}}), None, Draft::Draft202012 ; "ref_by_pointer_past_boolean_items")]
#[test_case(EvaluateContentDraft6::evaluate, serde_json::json!({"contentEncoding": "base64", "contentMediaType": "application/json"}), None, Draft::Draft6 ; "content_draft6")]
#[test_case(EvaluateContentEncodingDraft6::evaluate, serde_json::json!({"contentEncoding": "base64"}), None, Draft::Draft6 ; "content_encoding_draft6")]
#[test_case(EvaluateContentMediaTypeDraft6::evaluate, serde_json::json!({"contentMediaType": "application/json"}), None, Draft::Draft6 ; "content_media_type_draft6")]
#[test_case(EvaluateRefCycleOnOneInstance::evaluate, serde_json::json!({"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"minimum": 1, "allOf": [{"$ref": "#/$defs/node"}]}}}), None, Draft::Draft202012 ; "ref_cycle_on_one_instance")]
#[test_case(EvaluateFailingRefCycleOnOneInstance::evaluate, serde_json::json!({"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"maximum": 1, "allOf": [{"$ref": "#/$defs/node"}]}}}), None, Draft::Draft202012 ; "failing_ref_cycle_on_one_instance")]
#[test_case(EvaluateRefCycleThroughRoot::evaluate, serde_json::json!({"minimum": 1, "allOf": [{"$ref": "#"}]}), None, Draft::Draft202012 ; "ref_cycle_through_root")]
#[test_case(EvaluateRefCycleThroughInlineTarget::evaluate, serde_json::json!({"properties": {"b": {"maximum": 1, "allOf": [{"$ref": "#/properties/b"}]}}}), None, Draft::Draft202012 ; "ref_cycle_through_inline_target")]
#[test_case(EvaluateMutualRefCycleOnOneInstance::evaluate, serde_json::json!({"properties": {"b": {"$ref": "#/$defs/a"}}, "$defs": {"a": {"minimum": 1, "allOf": [{"$ref": "#/$defs/b"}]}, "b": {"maximum": 1, "allOf": [{"$ref": "#/$defs/a"}]}}}), None, Draft::Draft202012 ; "mutual_ref_cycle_on_one_instance")]
#[test_case(EvaluateRefCycleOnOneInstanceAndBelow::evaluate, serde_json::json!({"$ref": "#/$defs/node", "$defs": {"node": {"required": ["a"], "properties": {"c": {"$ref": "#/$defs/node"}}, "allOf": [{"$ref": "#/$defs/node"}]}}}), None, Draft::Draft202012 ; "ref_cycle_on_one_instance_and_below")]
#[test_case(EvaluateRefCycleThroughAnyOf::evaluate, serde_json::json!({"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"maximum": 1, "anyOf": [{"$ref": "#/$defs/node"}, {"type": "string"}]}}}), None, Draft::Draft202012 ; "ref_cycle_through_any_of")]
#[test_case(EvaluateRefCycleThroughOneOf::evaluate, serde_json::json!({"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"maximum": 1, "oneOf": [{"$ref": "#/$defs/node"}, {"type": "string"}]}}}), None, Draft::Draft202012 ; "ref_cycle_through_one_of")]
#[test_case(EvaluateRefCycleThroughIf::evaluate, serde_json::json!({"properties": {"b": {"$ref": "#/$defs/node"}}, "$defs": {"node": {"maximum": 1, "if": {"$ref": "#/$defs/node"}, "then": {"title": "t"}, "else": {"title": "e"}}}}), None, Draft::Draft202012 ; "ref_cycle_through_if")]
#[test_case(EvaluateRefCycleThroughRootAndAnyOf::evaluate, serde_json::json!({"maximum": 1, "anyOf": [{"$ref": "#"}, {"type": "string"}]}), None, Draft::Draft202012 ; "ref_cycle_through_root_and_any_of")]
#[test_case(EvaluateRefCycleThroughAnyOfBelow::evaluate, serde_json::json!({"$ref": "#/$defs/node", "$defs": {"node": {"properties": {"c": {"$ref": "#/$defs/node"}}, "anyOf": [{"$ref": "#/$defs/node"}, {"required": ["x"]}], "minProperties": 2}}}), None, Draft::Draft202012 ; "ref_cycle_through_any_of_below")]
fn test_evaluate_matches_runtime(
    evaluate: EvaluateFn,
    schema: serde_json::Value,
    base_uri: Option<&str>,
    draft: Draft,
) {
    let mut options = jsonschema::options().with_draft(draft);
    if let Some(base_uri) = base_uri {
        options = options.with_base_uri(base_uri);
    }
    let runtime = options.build(&schema).expect("valid schema");
    assert_evaluations_match(evaluate, &runtime);
}

#[test_case(EvaluateExternalRef::evaluate, serde_json::json!({"properties": {"a": {"$ref": "http://example.com/other.json"}, "b": {"$ref": "http://example.com/other.json#/$defs/s"}}}), &[("http://example.com/other.json", OTHER)], Draft::Draft202012 ; "external_ref")]
#[test_case(EvaluateEmbeddedId::evaluate, serde_json::json!({"$id": "http://example.com/root.json", "properties": {"foo": {"$id": "foo.json", "type": "object", "properties": {"bar": {"$ref": "#/$defs/s"}}, "$defs": {"s": {"minimum": 3}}}}}), &[], Draft::Draft202012 ; "embedded_id")]
#[test_case(EvaluateEmbeddedIdWithoutRootId::evaluate, serde_json::json!({"properties": {"a": {"$id": "http://example.com/a.json", "type": "integer", "minimum": 2}}}), &[], Draft::Draft202012 ; "embedded_id_without_root_id")]
#[test_case(EvaluateRefToEmbeddedResource::evaluate, serde_json::json!({"$defs": {"x": {"$id": "http://example.com/x", "type": "string"}}, "properties": {"a": {"$ref": "http://example.com/x"}}}), &[], Draft::Draft202012 ; "ref_to_embedded_resource")]
#[test_case(EvaluateRefByPointerIntoEmbeddedResource::evaluate, serde_json::json!({"$id": "http://example.com/root", "properties": {"a": {"$ref": "#/$defs/x/properties/y"}}, "$defs": {"x": {"$id": "x", "properties": {"y": {"type": "integer"}}}}}), &[], Draft::Draft202012 ; "ref_by_pointer_into_embedded_resource")]
#[test_case(EvaluateEmbeddedIdDraft4::evaluate, serde_json::json!({"properties": {"a": {"id": "http://example.com/a.json", "type": "object", "properties": {"b": {"$ref": "#/definitions/s"}}, "definitions": {"s": {"minimum": 3}}}}}), &[], Draft::Draft4 ; "embedded_id_draft4")]
#[test_case(EvaluateFolderId::evaluate, serde_json::json!({"$id": "http://localhost:1234/scope_change_defs1.json", "type": "object", "properties": {"list": {"$ref": "baseUriChangeFolder/"}}, "$defs": {"baz": {"$id": "baseUriChangeFolder/", "type": "array", "items": {"$ref": "folderInteger.json"}}}}), &[("http://localhost:1234/baseUriChangeFolder/folderInteger.json", r#"{"type": "integer"}"#)], Draft::Draft202012 ; "folder_id")]
#[test_case(EvaluateNoValidationVocabulary::evaluate, serde_json::json!({"$schema": "json-schema:///meta/no-validation", "properties": {"a": {"minimum": 10, "type": "string"}, "b": false}, "minLength": 2, "required": ["c"]}), &[("json-schema:///meta/no-validation", r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true}}"#)], Draft::Draft202012 ; "no_validation_vocabulary")]
#[test_case(EvaluateNoApplicatorVocabulary::evaluate, serde_json::json!({"$schema": "json-schema:///meta/no-applicator", "type": ["array", "object"], "items": {"type": "string"}, "contains": {"type": "integer"}, "properties": {"a": false}}), &[("json-schema:///meta/no-applicator", r#"{"$id":"json-schema:///meta/no-applicator","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/validation":true}}"#)], Draft::Draft202012 ; "no_applicator_vocabulary")]
#[test_case(EvaluateOptionalUnknownVocabulary::evaluate, serde_json::json!({"$schema": "json-schema:///meta/optional", "type": "number", "minimum": 2}), &[("json-schema:///meta/optional", r#"{"$id":"json-schema:///meta/optional","$schema":"https://json-schema.org/draft/2019-09/schema","$vocabulary":{"https://json-schema.org/draft/2019-09/vocab/core":true,"https://json-schema.org/draft/2019-09/vocab/validation":true,"json-schema:///vocab/custom":false}}"#)], Draft::Draft201909 ; "optional_unknown_vocabulary")]
#[test_case(EvaluateNoValidationVocabularyDraft201909::evaluate, serde_json::json!({"$schema": "json-schema:///meta/no-validation-2019", "properties": {"a": {"minimum": 10}}, "unevaluatedProperties": {"type": "string"}}), &[("json-schema:///meta/no-validation-2019", r#"{"$id":"json-schema:///meta/no-validation-2019","$schema":"https://json-schema.org/draft/2019-09/schema","$vocabulary":{"https://json-schema.org/draft/2019-09/vocab/core":true,"https://json-schema.org/draft/2019-09/vocab/applicator":true}}"#)], Draft::Draft201909 ; "no_validation_vocabulary_draft2019_09")]
#[test_case(EvaluateNoUnevaluatedVocabulary::evaluate, serde_json::json!({"$schema": "json-schema:///meta/no-unevaluated", "properties": {"a": {"type": "string"}}, "unevaluatedProperties": false, "unevaluatedItems": false}), &[("json-schema:///meta/no-unevaluated", r#"{"$id":"json-schema:///meta/no-unevaluated","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true}}"#)], Draft::Draft202012 ; "no_unevaluated_vocabulary")]
#[test_case(EvaluateNoContentVocabulary::evaluate, serde_json::json!({"$schema": "json-schema:///meta/no-content", "type": "string", "contentEncoding": "base64", "contentMediaType": "application/json", "contentSchema": {"type": "integer"}}), &[("json-schema:///meta/no-content", r#"{"$id":"json-schema:///meta/no-content","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true,"https://json-schema.org/draft/2020-12/vocab/validation":true}}"#)], Draft::Draft202012 ; "no_content_vocabulary")]
#[test_case(EvaluateRefToEarlierDraft::evaluate, serde_json::json!({"$ref": "http://example.com/ignore-prefix-items.json", "type": "array"}), &[("http://example.com/ignore-prefix-items.json", r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"http://example.com/ignore-prefix-items.json","prefixItems":[{"type":"integer"}]}"#)], Draft::Draft202012 ; "ref_to_earlier_draft")]
#[test_case(EvaluateRefToLaterDraft::evaluate, serde_json::json!({"allOf": [{"properties": {"foo": true}}, {"$ref": "http://example.com/dependent-required.json"}], "type": "object"}), &[("http://example.com/dependent-required.json", r#"{"$schema":"https://json-schema.org/draft/2019-09/schema","$id":"http://example.com/dependent-required.json","dependentRequired":{"foo":["bar"]}}"#)], Draft::Draft7 ; "ref_to_later_draft")]
#[test_case(EvaluateRefToDraft7Siblings::evaluate, serde_json::json!({"$ref": "http://example.com/draft7.json"}), &[("http://example.com/draft7.json", r##"{"$schema":"http://json-schema.org/draft-07/schema#","$id":"http://example.com/draft7.json","definitions":{"s":{"type":"string"}},"properties":{"a":{"$ref":"#/definitions/s","minLength":3}}}"##)], Draft::Draft202012 ; "ref_to_draft7_siblings")]
#[test_case(EvaluateNoValidationVocabularyThroughRef::evaluate, serde_json::json!({"$schema": "json-schema:///meta/no-validation", "properties": {"a": {"$ref": "#/$defs/x"}}, "$defs": {"x": {"minimum": 10, "properties": {"b": false}}}}), &[("json-schema:///meta/no-validation", r#"{"$id":"json-schema:///meta/no-validation","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/applicator":true}}"#)], Draft::Draft202012 ; "no_validation_vocabulary_through_ref")]
#[test_case(EvaluateUrnBase::evaluate, serde_json::json!({"$id": "urn:uuid:deadbeef-1234-ffff-ffff-4321feebdaed", "properties": {"a": {"$ref": "urn:uuid:deadbeef-1234-ffff-ffff-4321feebdaed#/$defs/s"}}, "$defs": {"s": {"type": "string"}}}), &[], Draft::Draft202012 ; "urn_base")]
#[test_case(EvaluateResourcesUnderDefaultBase::evaluate, serde_json::json!({"$id": "http://example.com/root", "properties": {"a": {"$ref": "json-schema:///other"}, "b": {"$ref": "json-schema:///other#/$defs/s"}, "c": {"$ref": "json-schema:///never"}, "d": {"$id": "json-schema:///inner", "maxLength": 0}}, "propertyNames": {"$ref": "json-schema:///names"}}), &[("json-schema:///other", r#"{"maxLength": 0, "$defs": {"s": {"minimum": 1}}}"#), ("json-schema:///never", "false"), ("json-schema:///names", r#"{"maxLength": 1}"#)], Draft::Draft202012 ; "resources_under_default_base")]
fn test_evaluate_with_resources_matches_runtime(
    evaluate: EvaluateFn,
    schema: serde_json::Value,
    resources: &[(&str, &str)],
    draft: Draft,
) {
    let contents: Vec<(&str, serde_json::Value)> = resources
        .iter()
        .map(|(uri, schema)| (*uri, serde_json::from_str(schema).expect("valid JSON")))
        .collect();
    let mut registry = jsonschema::Registry::new();
    for (uri, schema) in &contents {
        registry = registry
            .add(*uri, schema)
            .expect("resource should be accepted");
    }
    let registry = registry.prepare().expect("registry builds");
    let runtime = jsonschema::options()
        .with_draft(draft)
        .with_registry(&registry)
        .build(&schema)
        .expect("valid schema");
    assert_evaluations_match(evaluate, &runtime);
}

#[test_case(EvaluateCustomKeyword::evaluate, serde_json::json!({"type": "integer", "even": true, "minimum": 2, "title": "t"}), |options| options.with_keyword("even", even_factory) ; "custom_keyword")]
#[test_case(EvaluateCustomKeywordErrors::evaluate, serde_json::json!({"properties": {"a": {"multi": true}}, "multi": true}), |options| options.with_keyword("multi", multi_error_factory) ; "custom_keyword_errors")]
#[test_case(EvaluateCustomKeywordKnownName::evaluate, serde_json::json!({"description": "strings-bad", "title": "t"}), |options| options.with_keyword("description", string_gate_factory) ; "custom_keyword_known_name")]
#[test_case(EvaluateCustomFormat::evaluate, serde_json::json!({"type": "string", "format": "currency"}), |options| options.with_draft(Draft::Draft7).with_format("currency", is_currency_format) ; "custom_format")]
#[test_case(EvaluateNumberFormat::evaluate, serde_json::json!({"format": "even", "items": {"format": "even"}}), |options| options.with_draft(Draft::Draft7).with_number_format("even", is_even_number) ; "number_format")]
#[test_case(EvaluateIntegerNumberFormat::evaluate, serde_json::json!({"type": "integer", "format": "even"}), |options| options.with_draft(Draft::Draft7).with_number_format("even", is_even_number) ; "integer_number_format")]
#[test_case(EvaluateCustomFormatAsserted::evaluate, serde_json::json!({"type": "string", "format": "currency"}), |options| options.with_draft(Draft::Draft201909).with_format("currency", is_currency_format).should_validate_formats(true) ; "custom_format_asserted")]
#[test_case(EvaluateCustomFormatOverride::evaluate, serde_json::json!({"type": "string", "format": "email"}), |options| options.with_draft(Draft::Draft7).with_format("email", is_literal_x).should_validate_formats(true) ; "custom_format_override")]
#[test_case(EvaluateIgnoredUnknownFormat::evaluate, serde_json::json!({"type": "string", "format": "made-up"}), |options| options.with_draft(Draft::Draft7).should_validate_formats(true).should_ignore_unknown_formats(true) ; "ignored_unknown_format")]
#[test_case(EvaluateCustomContentMediaType::evaluate, serde_json::json!({"contentMediaType": "text/hex"}), |options| options.with_draft(Draft::Draft7).with_content_media_type("text/hex", is_hex_content) ; "custom_content_media_type")]
#[test_case(EvaluateCustomContentEncoding::evaluate, serde_json::json!({"contentEncoding": "prefixed", "contentMediaType": "application/json"}), |options| options.with_draft(Draft::Draft7).with_content_encoding("prefixed", check_prefixed, convert_prefixed) ; "custom_content_encoding")]
fn test_evaluate_with_options_matches_runtime(
    evaluate: EvaluateFn,
    schema: serde_json::Value,
    configure: fn(jsonschema::ValidationOptions) -> jsonschema::ValidationOptions,
) {
    let runtime = configure(jsonschema::options())
        .build(&schema)
        .expect("valid schema");
    assert_evaluations_match(evaluate, &runtime);
}

// Enough units below the same `$ref`s to cache their evaluation paths, each `$ref` below two others.
#[jsonschema::validator(
    schema = r##"{"$defs": {"node": {"properties": {"v": {"$ref": "#/$defs/leaf"}, "w": {"$ref": "#/$defs/node/$defs/inner"}}, "$defs": {"inner": {"$ref": "#/$defs/leaf"}}}, "leaf": {"type": "integer", "minimum": 0}}, "items": {"allOf": [{"$ref": "#/$defs/node"}, {"$ref": "#/$defs/node"}]}}"##
)]
struct EvaluateManyRefVisits;

#[test]
fn test_evaluate_matches_runtime_with_cached_evaluation_paths() {
    let schema = serde_json::json!({"$defs": {"node": {"properties": {"v": {"$ref": "#/$defs/leaf"}, "w": {"$ref": "#/$defs/node/$defs/inner"}}, "$defs": {"inner": {"$ref": "#/$defs/leaf"}}}, "leaf": {"type": "integer", "minimum": 0}}, "items": {"allOf": [{"$ref": "#/$defs/node"}, {"$ref": "#/$defs/node"}]}});
    let runtime = jsonschema::validator_for(&schema).expect("valid schema");
    let instance = serde_json::Value::Array(
        (0..3000_i64)
            .map(|index| serde_json::json!({"v": index % 7 - 1, "w": index % 5 - 1}))
            .collect(),
    );
    assert_eq!(
        evaluation_outputs(&EvaluateManyRefVisits::evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

// `dependencies` takes the errors of the generated `iter_errors`: more than a few, and items with
// none.
#[jsonschema::validator(
    schema = r#"{"dependencies": {"a": ["b"]}, "items": {"minimum": 10}}"#,
    draft = Draft7
)]
struct EvaluateManyErrorsBesideDependencies;

#[test]
fn test_evaluate_matches_runtime_with_many_errors_beside_dependencies() {
    let schema = serde_json::json!({"dependencies": {"a": ["b"]}, "items": {"minimum": 10}});
    let runtime = jsonschema::draft7::new(&schema).expect("valid schema");
    let instance = serde_json::Value::Array((0..20_i64).map(serde_json::Value::from).collect());
    assert_eq!(
        evaluation_outputs(&EvaluateManyErrorsBesideDependencies::evaluate(&instance)),
        evaluation_outputs(&runtime.evaluate(&instance)),
    );
}

#[jsonschema::validator(schema = r#"{"properties": {"a": true, "b": {"type": "integer"}}}"#)]
struct EvaluatePropertiesAnnotation;

// The names `properties` annotates read the same however the annotation is first used.
#[test]
fn test_evaluate_properties_annotation_matches_runtime() {
    let runtime = jsonschema::validator_for(
        &serde_json::json!({"properties": {"a": true, "b": {"type": "integer"}}}),
    )
    .expect("valid schema");
    let instance = serde_json::json!({"a": 1, "b": 2, "c": 3});
    let names = serde_json::json!(["a", "b"]);
    let runtime_evaluation = runtime.evaluate(&instance);
    let expected: Vec<_> = runtime_evaluation
        .iter_annotations()
        .map(|entry| entry.annotations)
        .collect();

    let evaluation = EvaluatePropertiesAnnotation::evaluate(&instance);
    let serialized: Vec<_> = evaluation
        .iter_annotations()
        .map(|entry| entry.annotations)
        .collect();
    assert_eq!(
        serde_json::to_value(&serialized).expect("serializable"),
        serde_json::to_value(&expected).expect("serializable")
    );

    let evaluation = EvaluatePropertiesAnnotation::evaluate(&instance);
    let compared: Vec<_> = evaluation
        .iter_annotations()
        .map(|entry| entry.annotations.clone())
        .collect();
    assert_eq!(compared.iter().collect::<Vec<_>>(), expected);
    assert_eq!(
        format!("{:?}", compared.iter().collect::<Vec<_>>()),
        format!("{expected:?}")
    );
    let shared = compared.clone();
    let while_shared: Vec<_> = compared
        .into_iter()
        .map(jsonschema::output::Annotations::into_inner)
        .collect();
    assert_eq!(while_shared, std::slice::from_ref(&names));
    let once_unique: Vec<_> = shared
        .into_iter()
        .map(jsonschema::output::Annotations::into_inner)
        .collect();
    assert_eq!(once_unique, std::slice::from_ref(&names));

    let unread: Vec<_> = EvaluatePropertiesAnnotation::evaluate(&instance)
        .iter_annotations()
        .map(|entry| entry.annotations.clone().into_inner())
        .collect();
    assert_eq!(unread, [names]);
}

fn assert_evaluations_match(evaluate: EvaluateFn, runtime: &jsonschema::Validator) {
    for instance in [
        serde_json::json!(null),
        serde_json::json!({"a": [1, "x"]}),
        serde_json::json!(5),
        serde_json::json!("a"),
        serde_json::json!("ab"),
        serde_json::json!("abcd"),
        serde_json::json!(0),
        serde_json::json!(1),
        serde_json::json!(2),
        serde_json::json!(6),
        serde_json::json!([1, 1]),
        serde_json::json!([1, 2, 3]),
        serde_json::json!({}),
        serde_json::json!({"a": 1, "b": 2, "c": 3}),
        serde_json::json!("a@b.c"),
        serde_json::json!("MQ=="),
        serde_json::json!("ew=="),
        serde_json::json!("!!"),
        serde_json::json!({"a": "x", "b": 1, "c": 2, "d": null, "e~/f": "", "s p": 1, "^a": {"b": 1}}),
        serde_json::json!({"^a": {"b": 3, "c": 1}, "s p": "x", "e~/f": "y"}),
        serde_json::json!({"a": 1, "b": "x", "xy": "q", "x": "", "c": 2, "ab": "3", "12": 1, "3": 5, "a/b~": "n", " ": 0}),
        serde_json::json!({"c": "x", "ca": 1, "d": {"a": 1}, "e": {"b": 2}}),
        serde_json::json!({"a": {"bb": 1, "c": 2}, "x@y.z": 1, "B": 2}),
        serde_json::json!({"a": 1, "c": 2, "e": 3}),
        serde_json::json!({"a": 1, "x": 1, "d": 1, "f": 1}),
        serde_json::json!([]),
        serde_json::json!(["x"]),
        serde_json::json!([1, "a", 3]),
        serde_json::json!([1, 2, "a", "b"]),
        serde_json::json!([{"a": 1}, {"a": "x"}, 5]),
        serde_json::json!([1, 1, 1, 1, "x"]),
        serde_json::json!([{"a": 2}, {"b": 1}, {"a": 3}]),
        serde_json::json!({"a": 1, "b": 1}),
        serde_json::json!({"b": 2}),
        serde_json::json!(true),
        serde_json::json!({"foo": "x"}),
        serde_json::json!({"next": {"next": {"v": "x"}}}),
        serde_json::json!({"next": {"v": 1}, "v": 2}),
        serde_json::json!({"a": 1, "b": "x"}),
        serde_json::json!({"abcd": 1, "ab": 2}),
        serde_json::json!({"a": 1, "c": {"a": 1, "c": {"b": 2, "c": 5}}}),
        serde_json::json!({"a": 1, "c": {"b": 1, "c": {"a": 1, "c": "x"}}}),
        // More members than the runtime scans before looking declared names up instead.
        serde_json::json!({"a": 1, "b": "x", "c": 1, "d": 1, "e": 1, "f": 1, "g": 1, "h": 1, "i": 1, "j": 1}),
        serde_json::json!({"foo": {"bar": 1}}),
        serde_json::json!({"foo": {"bar": 5}}),
        serde_json::json!({"a": {"n": 1}, "b": 2}),
        serde_json::json!({"a": {"n": "s"}, "b": "x"}),
        serde_json::json!({"a": {"y": "x", "b": 1}}),
        serde_json::json!({"a": {"b": 4}}),
        serde_json::json!({"x1": 1, "x2": 5, "a": 1}),
        serde_json::json!({"o": {"p": 1, "q": 2}}),
        serde_json::json!({"a": "s", "b": 1, "c": 2}),
        serde_json::json!({"a": 1, "c": 1}),
        serde_json::json!(["s", 1, 2]),
        serde_json::json!([1, "a", "b"]),
        serde_json::json!(["a", "b", 3]),
        serde_json::json!({"list": [1, 2]}),
        serde_json::json!({"list": ["a"]}),
        serde_json::json!(["foo", 42]),
        serde_json::json!(["foo", "bar"]),
        serde_json::json!({"t": {"child": {"marker": 1}}}),
        serde_json::json!({"t": {"child": {"marker": true}}}),
        serde_json::json!({"t": {"child": {"t": {"child": {"marker": "x"}}}}}),
        serde_json::json!({"t": {"child": {"marker": 1, "ok": 1}, "ok": 1}}),
        serde_json::json!({"foo": "x", "bar": "y"}),
        serde_json::json!({"foo": "x", "bar": "y", "baz": 1}),
        serde_json::json!({"l": [1]}),
        serde_json::json!({"l": [{}]}),
        serde_json::json!({"foo": 1, "bar": 2}),
        serde_json::json!({"a": "xyzw"}),
        serde_json::json!({"a": 1}),
        serde_json::json!({"a": {"b": 1}}),
        serde_json::json!({"kind": "a", "v": 1}),
        serde_json::json!({"kind": "a", "v": "x"}),
        serde_json::json!({"kind": "b", "v": "x"}),
        serde_json::json!({"kind": "c", "v": 1, "w": 2}),
        serde_json::json!({"kind": "a", "v": 1, "w": 2}),
        serde_json::json!({"kind": "z"}),
        serde_json::json!({"kind": 1}),
        serde_json::json!(3),
        serde_json::json!(200),
        serde_json::json!("$12"),
        serde_json::json!("$"),
        serde_json::json!("x"),
        serde_json::json!("a@b"),
        serde_json::json!("0aF"),
        serde_json::json!("zz"),
        serde_json::json!("p:{}"),
        serde_json::json!("p:{"),
        serde_json::json!("q"),
        serde_json::json!({"c": {"d": {"next": {"marker": 1}}}}),
        serde_json::json!({"c": {"d": {"next": {"marker": true}}, "x": 1}}),
        serde_json::json!([1, 2.5, 3]),
        serde_json::json!([1, "x", 3, 4]),
        serde_json::json!(["x", 1, 2, "y"]),
        serde_json::json!(["a", 1, "b", 3]),
        serde_json::json!([1, 2, 7, 3]),
        serde_json::json!([1, "a", "b", 2, "c"]),
        serde_json::json!([1, 2, 0, 3, 4]),
        serde_json::json!(["a@b.c", "x"]),
        serde_json::json!([[1, 2], [], [3, "x", 4]]),
        serde_json::json!([[1, 2], [3, 4, 5], [6, "x"]]),
        serde_json::json!({"a": [1, 2], "b": [3, "x", 4]}),
        serde_json::json!({"a": [1, 2], "b": [3, 4]}),
    ] {
        assert_eq!(
            evaluation_outputs(&evaluate(&instance)),
            evaluation_outputs(&runtime.evaluate(&instance)),
        );
    }
}

// `inner` is the schema every reference below names; `outer` is the outermost `item` in scope.
#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"inner#item","$defs":{"outer":{"$dynamicAnchor":"item","type":"null"},"inner":{"$id":"inner","$dynamicAnchor":"item","type":"string"}}}"##
)]
struct RefToDynamicAnchorValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicRef":"inner#item","$defs":{"outer":{"$dynamicAnchor":"item","type":"null"},"inner":{"$id":"inner","$dynamicAnchor":"item","type":"string"}}}"##
)]
struct DynamicRefToDynamicAnchorValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","properties":{"static":{"$ref":"inner#item"},"dynamic":{"$dynamicRef":"inner#item"}},"$defs":{"outer":{"$dynamicAnchor":"item","type":"null"},"inner":{"$id":"inner","$dynamicAnchor":"item","type":"string"}}}"##
)]
struct RefBesideDynamicRefValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$ref":"inner#item","unevaluatedProperties":false,"unevaluatedItems":false,"$defs":{"outer":{"$dynamicAnchor":"item","properties":{"outer":true},"prefixItems":[true]},"inner":{"$id":"inner","$dynamicAnchor":"item","properties":{"inner":true},"prefixItems":[true,true]}}}"##
)]
struct RefToDynamicAnchorUnevaluatedValidator;

#[jsonschema::validator(
    schema = r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/root","$dynamicRef":"inner#item","unevaluatedProperties":false,"unevaluatedItems":false,"$defs":{"outer":{"$dynamicAnchor":"item","properties":{"outer":true},"prefixItems":[true]},"inner":{"$id":"inner","$dynamicAnchor":"item","properties":{"inner":true},"prefixItems":[true,true]}}}"##
)]
struct DynamicRefToDynamicAnchorUnevaluatedValidator;

#[test_case(RefToDynamicAnchorValidator::is_valid, serde_json::json!("a"), serde_json::json!(null); "ref is static")]
#[test_case(DynamicRefToDynamicAnchorValidator::is_valid, serde_json::json!(null), serde_json::json!("a"); "dynamic ref follows the scope")]
#[test_case(RefBesideDynamicRefValidator::is_valid, serde_json::json!({"static": "a", "dynamic": null}), serde_json::json!({"static": null}); "ref beside dynamic ref, static side")]
#[test_case(RefBesideDynamicRefValidator::is_valid, serde_json::json!({"static": "a", "dynamic": null}), serde_json::json!({"dynamic": "a"}); "ref beside dynamic ref, dynamic side")]
#[test_case(RefToDynamicAnchorUnevaluatedValidator::is_valid, serde_json::json!({"inner": 1}), serde_json::json!({"outer": 1}); "ref, unevaluated properties")]
#[test_case(RefToDynamicAnchorUnevaluatedValidator::is_valid, serde_json::json!([1, 2]), serde_json::json!([1, 2, 3]); "ref, unevaluated items")]
#[test_case(DynamicRefToDynamicAnchorUnevaluatedValidator::is_valid, serde_json::json!({"outer": 1}), serde_json::json!({"inner": 1}); "dynamic ref, unevaluated properties")]
#[test_case(DynamicRefToDynamicAnchorUnevaluatedValidator::is_valid, serde_json::json!([1]), serde_json::json!([1, 2]); "dynamic ref, unevaluated items")]
fn ref_to_dynamic_anchor_ignores_dynamic_scope(
    is_valid: fn(&serde_json::Value) -> bool,
    valid: serde_json::Value,
    invalid: serde_json::Value,
) {
    assert!(is_valid(&valid));
    assert!(!is_valid(&invalid));
}
