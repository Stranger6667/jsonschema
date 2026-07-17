use jsonschema::{
    error::{TypeKind, ValidationErrorKind},
    paths::{Location, LocationSegment},
    ValidationError,
};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum PathSegment {
    Key(String),
    Index(usize),
}

fn location_to_path(location: &Location) -> Vec<PathSegment> {
    location
        .into_iter()
        .map(|segment| match segment {
            LocationSegment::Property(p) => PathSegment::Key(p.to_string()),
            LocationSegment::Index(i) => PathSegment::Index(i),
        })
        .collect()
}

fn type_names(kind: &TypeKind) -> Vec<String> {
    match kind {
        TypeKind::Single(t) => vec![t.as_str().to_string()],
        TypeKind::Multiple(set) => set.iter().map(|t| t.as_str().to_string()).collect(),
    }
}

fn error_context(context: &[Vec<ValidationError<'static>>]) -> Vec<Vec<Error>> {
    context
        .iter()
        .map(|branch| branch.iter().map(Error::from).collect())
        .collect()
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum Kind {
    AdditionalItems { limit: usize },
    AdditionalProperties { unexpected: Vec<String> },
    AnyOf { context: Vec<Vec<Error>> },
    BacktrackLimitExceeded { message: String },
    RegexEngineFailure { message: String },
    Constant { expected_value: Value },
    Contains,
    ContentEncoding { content_encoding: String },
    ContentMediaType { content_media_type: String },
    Custom { keyword: String, message: String },
    Enum { options: Value },
    ExclusiveMaximum { limit: Value },
    ExclusiveMinimum { limit: Value },
    FalseSchema,
    Format { format: String },
    FromUtf8 { message: String },
    MaxItems { limit: u64 },
    Maximum { limit: Value },
    MaxLength { limit: u64 },
    MaxProperties { limit: u64 },
    MinItems { limit: u64 },
    Minimum { limit: Value },
    MinLength { limit: u64 },
    MinProperties { limit: u64 },
    MultipleOf { multiple_of: Value },
    Not { schema: Value },
    OneOfMultipleValid { context: Vec<Vec<Error>> },
    OneOfNotValid { context: Vec<Vec<Error>> },
    Pattern { pattern: String },
    PropertyNames { error: Box<Error> },
    Required { property: Value },
    Type { types: Vec<String> },
    UnevaluatedItems { unexpected: Vec<String> },
    UnevaluatedProperties { unexpected: Vec<String> },
    UniqueItems,
    Referencing { message: String },
}

impl From<&ValidationErrorKind> for Kind {
    fn from(kind: &ValidationErrorKind) -> Self {
        match kind {
            ValidationErrorKind::AdditionalItems { limit } => {
                Kind::AdditionalItems { limit: *limit }
            }
            ValidationErrorKind::AdditionalProperties { unexpected } => {
                Kind::AdditionalProperties {
                    unexpected: unexpected.clone(),
                }
            }
            ValidationErrorKind::AnyOf { context } => Kind::AnyOf {
                context: error_context(context),
            },
            ValidationErrorKind::BacktrackLimitExceeded { error } => Kind::BacktrackLimitExceeded {
                message: error.to_string(),
            },
            ValidationErrorKind::RegexEngineFailure { message } => Kind::RegexEngineFailure {
                message: message.clone(),
            },
            ValidationErrorKind::Constant { expected_value } => Kind::Constant {
                expected_value: expected_value.clone(),
            },
            ValidationErrorKind::Contains => Kind::Contains,
            ValidationErrorKind::ContentEncoding { content_encoding } => Kind::ContentEncoding {
                content_encoding: content_encoding.clone(),
            },
            ValidationErrorKind::ContentMediaType { content_media_type } => {
                Kind::ContentMediaType {
                    content_media_type: content_media_type.clone(),
                }
            }
            ValidationErrorKind::Custom { keyword, message } => Kind::Custom {
                keyword: keyword.clone(),
                message: message.clone(),
            },
            ValidationErrorKind::Enum { options } => Kind::Enum {
                options: options.clone(),
            },
            ValidationErrorKind::ExclusiveMaximum { limit } => Kind::ExclusiveMaximum {
                limit: limit.clone(),
            },
            ValidationErrorKind::ExclusiveMinimum { limit } => Kind::ExclusiveMinimum {
                limit: limit.clone(),
            },
            ValidationErrorKind::FalseSchema => Kind::FalseSchema,
            ValidationErrorKind::Format { format } => Kind::Format {
                format: format.clone(),
            },
            ValidationErrorKind::FromUtf8 { error } => Kind::FromUtf8 {
                message: error.to_string(),
            },
            ValidationErrorKind::MaxItems { limit } => Kind::MaxItems { limit: *limit },
            ValidationErrorKind::Maximum { limit } => Kind::Maximum {
                limit: limit.clone(),
            },
            ValidationErrorKind::MaxLength { limit } => Kind::MaxLength { limit: *limit },
            ValidationErrorKind::MaxProperties { limit } => Kind::MaxProperties { limit: *limit },
            ValidationErrorKind::MinItems { limit } => Kind::MinItems { limit: *limit },
            ValidationErrorKind::Minimum { limit } => Kind::Minimum {
                limit: limit.clone(),
            },
            ValidationErrorKind::MinLength { limit } => Kind::MinLength { limit: *limit },
            ValidationErrorKind::MinProperties { limit } => Kind::MinProperties { limit: *limit },
            ValidationErrorKind::MultipleOf { multiple_of } => Kind::MultipleOf {
                multiple_of: multiple_of.clone(),
            },
            ValidationErrorKind::Not { schema } => Kind::Not {
                schema: schema.clone(),
            },
            ValidationErrorKind::OneOfMultipleValid { context } => Kind::OneOfMultipleValid {
                context: error_context(context),
            },
            ValidationErrorKind::OneOfNotValid { context } => Kind::OneOfNotValid {
                context: error_context(context),
            },
            ValidationErrorKind::Pattern { pattern } => Kind::Pattern {
                pattern: pattern.clone(),
            },
            ValidationErrorKind::PropertyNames { error } => Kind::PropertyNames {
                error: Box::new(Error::from(error.as_ref())),
            },
            ValidationErrorKind::Required { property } => Kind::Required {
                property: property.clone(),
            },
            ValidationErrorKind::Type { kind } => Kind::Type {
                types: type_names(kind),
            },
            ValidationErrorKind::UnevaluatedItems { unexpected } => Kind::UnevaluatedItems {
                unexpected: unexpected.clone(),
            },
            ValidationErrorKind::UnevaluatedProperties { unexpected } => {
                Kind::UnevaluatedProperties {
                    unexpected: unexpected.clone(),
                }
            }
            ValidationErrorKind::UniqueItems => Kind::UniqueItems,
            // `Referencing` only originates from `$ref`/`$id` resolution during schema
            // compilation; `build()` returns it as an `Err` before any `Error` exists,
            // so this arm never runs against a value produced through the crate's public API.
            ValidationErrorKind::Referencing(error) => Kind::Referencing {
                message: error.to_string(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Error {
    pub(crate) message: String,
    pub(crate) instance_path: Vec<PathSegment>,
    pub(crate) schema_path: Vec<PathSegment>,
    pub(crate) kind: Kind,
}

impl From<&ValidationError<'_>> for Error {
    fn from(error: &ValidationError<'_>) -> Self {
        Error {
            message: error.to_string(),
            instance_path: location_to_path(error.instance_path()),
            schema_path: location_to_path(error.schema_path()),
            kind: Kind::from(error.kind()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonschema::{options, Draft, Keyword, PatternOptions, Validator};
    use serde_json::{json, Map, Value};
    use test_case::test_case;

    fn errors(schema: &Value, instance: &Value) -> Value {
        errors_of(&options().build(schema).unwrap(), instance)
    }

    fn errors_of(validator: &Validator, instance: &Value) -> Value {
        let errors: Vec<Error> = validator
            .iter_errors(instance)
            .map(|e| Error::from(&e))
            .collect();
        serde_json::to_value(&errors).unwrap()
    }

    #[test]
    fn nested_instance_path_is_array_of_segments() {
        let schema = json!({"properties": {"a": {"type": "array", "items": {"type": "string"}}}});
        let instance = json!({"a": [1]});
        assert_eq!(
            errors(&schema, &instance),
            json!([
                {
                    "message": "1 is not of type \"string\"",
                    "instancePath": ["a", 0],
                    "schemaPath": ["properties", "a", "items", "type"],
                    "kind": {"type": "type", "types": ["string"]}
                }
            ])
        );
    }

    #[test]
    fn any_of_nests_context() {
        let schema = json!({"anyOf": [{"type": "string"}, {"type": "number"}]});
        let instance = json!(true);
        assert_eq!(
            errors(&schema, &instance),
            json!([
                {
                    "message": "true is not valid under any of the schemas listed in the 'anyOf' keyword",
                    "instancePath": [],
                    "schemaPath": ["anyOf"],
                    "kind": {
                        "type": "anyOf",
                        "context": [
                            [
                                {
                                    "message": "true is not of type \"string\"",
                                    "instancePath": [],
                                    "schemaPath": ["anyOf", 0, "type"],
                                    "kind": {"type": "type", "types": ["string"]}
                                }
                            ],
                            [
                                {
                                    "message": "true is not of type \"number\"",
                                    "instancePath": [],
                                    "schemaPath": ["anyOf", 1, "type"],
                                    "kind": {"type": "type", "types": ["number"]}
                                }
                            ]
                        ]
                    }
                }
            ])
        );
    }

    #[test_case(
        &json!({"type": "string"}),
        &json!(42),
        &json!([
            {
                "message": "42 is not of type \"string\"",
                "instancePath": [],
                "schemaPath": ["type"],
                "kind": {"type": "type", "types": ["string"]}
            }
        ])
    )]
    #[test_case(
        &json!({"type": ["string", "number"]}),
        &json!(true),
        &json!([
            {
                "message": "true is not of types \"number\", \"string\"",
                "instancePath": [],
                "schemaPath": ["type"],
                "kind": {"type": "type", "types": ["number", "string"]}
            }
        ])
    )]
    fn type_error_lists_type_names(schema: &Value, instance: &Value, expected: &Value) {
        assert_eq!(errors(schema, instance), *expected);
    }

    #[test_case(
        &json!({"maximum": 10}),
        &json!(20),
        &json!([{"message": "20 is greater than the maximum of 10", "instancePath": [], "schemaPath": ["maximum"], "kind": {"type": "maximum", "limit": 10}}])
    )]
    #[test_case(
        &json!({"minimum": 10}),
        &json!(5),
        &json!([{"message": "5 is less than the minimum of 10", "instancePath": [], "schemaPath": ["minimum"], "kind": {"type": "minimum", "limit": 10}}])
    )]
    #[test_case(
        &json!({"exclusiveMaximum": 10}),
        &json!(10),
        &json!([{"message": "10 is greater than or equal to the maximum of 10", "instancePath": [], "schemaPath": ["exclusiveMaximum"], "kind": {"type": "exclusiveMaximum", "limit": 10}}])
    )]
    #[test_case(
        &json!({"exclusiveMinimum": 10}),
        &json!(10),
        &json!([{"message": "10 is less than or equal to the minimum of 10", "instancePath": [], "schemaPath": ["exclusiveMinimum"], "kind": {"type": "exclusiveMinimum", "limit": 10}}])
    )]
    #[test_case(
        &json!({"maxLength": 2}),
        &json!("abc"),
        &json!([{"message": "\"abc\" is longer than 2 characters", "instancePath": [], "schemaPath": ["maxLength"], "kind": {"type": "maxLength", "limit": 2}}])
    )]
    #[test_case(
        &json!({"minLength": 5}),
        &json!("ab"),
        &json!([{"message": "\"ab\" is shorter than 5 characters", "instancePath": [], "schemaPath": ["minLength"], "kind": {"type": "minLength", "limit": 5}}])
    )]
    #[test_case(
        &json!({"maxItems": 1}),
        &json!([1, 2]),
        &json!([{"message": "[1,2] has more than 1 item", "instancePath": [], "schemaPath": ["maxItems"], "kind": {"type": "maxItems", "limit": 1}}])
    )]
    #[test_case(
        &json!({"minItems": 2}),
        &json!([1]),
        &json!([{"message": "[1] has less than 2 items", "instancePath": [], "schemaPath": ["minItems"], "kind": {"type": "minItems", "limit": 2}}])
    )]
    #[test_case(
        &json!({"maxProperties": 1}),
        &json!({"a": 1, "b": 2}),
        &json!([{"message": "{\"a\":1,\"b\":2} has more than 1 property", "instancePath": [], "schemaPath": ["maxProperties"], "kind": {"type": "maxProperties", "limit": 1}}])
    )]
    #[test_case(
        &json!({"minProperties": 2}),
        &json!({"a": 1}),
        &json!([{"message": "{\"a\":1} has less than 2 properties", "instancePath": [], "schemaPath": ["minProperties"], "kind": {"type": "minProperties", "limit": 2}}])
    )]
    #[test_case(
        &json!({"required": ["a"]}),
        &json!({}),
        &json!([{"message": "\"a\" is a required property", "instancePath": [], "schemaPath": ["required"], "kind": {"type": "required", "property": "a"}}])
    )]
    #[test_case(
        &json!({"const": 1}),
        &json!(2),
        &json!([{"message": "1 was expected", "instancePath": [], "schemaPath": ["const"], "kind": {"type": "constant", "expected_value": 1}}])
    )]
    #[test_case(
        &json!({"enum": [1, 2, 3]}),
        &json!(4),
        &json!([{"message": "4 is not one of 1, 2 or 3", "instancePath": [], "schemaPath": ["enum"], "kind": {"type": "enum", "options": [1, 2, 3]}}])
    )]
    #[test_case(
        &json!({"uniqueItems": true}),
        &json!([1, 1]),
        &json!([{"message": "[1,1] has non-unique elements", "instancePath": [], "schemaPath": ["uniqueItems"], "kind": {"type": "uniqueItems"}}])
    )]
    #[test_case(
        &json!({"pattern": "^a"}),
        &json!("b"),
        &json!([{"message": "\"b\" does not match \"^a\"", "instancePath": [], "schemaPath": ["pattern"], "kind": {"type": "pattern", "pattern": "^a"}}])
    )]
    #[test_case(
        &json!({"multipleOf": 2}),
        &json!(3),
        &json!([{"message": "3 is not a multiple of 2", "instancePath": [], "schemaPath": ["multipleOf"], "kind": {"type": "multipleOf", "multiple_of": 2}}])
    )]
    #[test_case(
        &json!({"properties": {"a": {}}, "additionalProperties": false}),
        &json!({"a": 1, "b": 2}),
        &json!([{"message": "Additional properties are not allowed ('b' was unexpected)", "instancePath": [], "schemaPath": ["additionalProperties"], "kind": {"type": "additionalProperties", "unexpected": ["b"]}}])
    )]
    #[test_case(
        &json!({"type": "string"}),
        &json!(42),
        &json!([{"message": "42 is not of type \"string\"", "instancePath": [], "schemaPath": ["type"], "kind": {"type": "type", "types": ["string"]}}])
    )]
    #[test_case(
        &json!({"contains": {"type": "string"}}),
        &json!([1, 2, 3]),
        &json!([{"message": "None of [1,2,3] are valid under the given schema", "instancePath": [], "schemaPath": ["contains"], "kind": {"type": "contains"}}])
    )]
    #[test_case(
        &json!({"not": {"type": "string"}}),
        &json!("abc"),
        &json!([{"message": "{\"type\":\"string\"} is not allowed for \"abc\"", "instancePath": [], "schemaPath": ["not"], "kind": {"type": "not", "schema": {"type": "string"}}}])
    )]
    #[test_case(
        &json!({"anyOf": [{"type": "string"}, {"type": "number"}]}),
        &json!(true),
        &json!([
            {
                "message": "true is not valid under any of the schemas listed in the 'anyOf' keyword",
                "instancePath": [],
                "schemaPath": ["anyOf"],
                "kind": {
                    "type": "anyOf",
                    "context": [
                        [{"message": "true is not of type \"string\"", "instancePath": [], "schemaPath": ["anyOf", 0, "type"], "kind": {"type": "type", "types": ["string"]}}],
                        [{"message": "true is not of type \"number\"", "instancePath": [], "schemaPath": ["anyOf", 1, "type"], "kind": {"type": "type", "types": ["number"]}}]
                    ]
                }
            }
        ])
    )]
    #[test_case(
        &json!({"oneOf": [{"type": "string"}, {"type": "number"}]}),
        &json!(true),
        &json!([
            {
                "message": "true is not valid under any of the schemas listed in the 'oneOf' keyword",
                "instancePath": [],
                "schemaPath": ["oneOf"],
                "kind": {
                    "type": "oneOfNotValid",
                    "context": [
                        [{"message": "true is not of type \"string\"", "instancePath": [], "schemaPath": ["oneOf", 0, "type"], "kind": {"type": "type", "types": ["string"]}}],
                        [{"message": "true is not of type \"number\"", "instancePath": [], "schemaPath": ["oneOf", 1, "type"], "kind": {"type": "type", "types": ["number"]}}]
                    ]
                }
            }
        ])
    )]
    #[test_case(
        &json!({"oneOf": [{"minimum": 0}, {"maximum": 10}]}),
        &json!(5),
        &json!([
            {
                "message": "5 is valid under more than one of the schemas listed in the 'oneOf' keyword",
                "instancePath": [],
                "schemaPath": ["oneOf"],
                "kind": {"type": "oneOfMultipleValid", "context": [[], []]}
            }
        ])
    )]
    #[test_case(
        &json!({"propertyNames": {"pattern": "^[a-z]+$"}}),
        &json!({"A": 1}),
        &json!([
            {
                "message": "\"A\" does not match \"^[a-z]+$\"",
                "instancePath": [],
                "schemaPath": ["propertyNames", "pattern"],
                "kind": {
                    "type": "propertyNames",
                    "error": {
                        "message": "\"A\" does not match \"^[a-z]+$\"",
                        "instancePath": [],
                        "schemaPath": ["propertyNames", "pattern"],
                        "kind": {"type": "pattern", "pattern": "^[a-z]+$"}
                    }
                }
            }
        ])
    )]
    #[test_case(
        &json!({"format": "email"}),
        &json!("not-an-email"),
        &json!([{"message": "\"not-an-email\" is not a \"email\"", "instancePath": [], "schemaPath": ["format"], "kind": {"type": "format", "format": "email"}}])
    )]
    #[test_case(
        &json!({"properties": {"a": false}}),
        &json!({"a": 1}),
        &json!([{"message": "False schema does not allow 1", "instancePath": ["a"], "schemaPath": ["properties", "a"], "kind": {"type": "falseSchema"}}])
    )]
    #[test_case(
        &json!({"properties": {"a": {}}, "unevaluatedProperties": false}),
        &json!({"a": 1, "b": 2}),
        &json!([{"message": "Unevaluated properties are not allowed ('b' was unexpected)", "instancePath": [], "schemaPath": ["unevaluatedProperties"], "kind": {"type": "unevaluatedProperties", "unexpected": ["b"]}}])
    )]
    #[test_case(
        &json!({"prefixItems": [{}], "unevaluatedItems": false}),
        &json!([1, 2]),
        &json!([{"message": "Unevaluated items are not allowed ('2' was unexpected)", "instancePath": [], "schemaPath": ["unevaluatedItems"], "kind": {"type": "unevaluatedItems", "unexpected": ["2"]}}])
    )]
    fn kind_type_mapping(schema: &Value, instance: &Value, expected: &Value) {
        let validator = options()
            .should_validate_formats(true)
            .build(schema)
            .unwrap();
        assert_eq!(errors_of(&validator, instance), *expected);
    }

    // `items` as a tuple array + `additionalItems` is a draft <= 7 construct; draft 2019-09+
    // uses `prefixItems`/`unevaluatedItems` instead, so this needs its own non-default options.
    #[test]
    fn additional_items_maps_to_additional_items_kind() {
        let schema = json!({"items": [{"type": "string"}], "additionalItems": false});
        let validator = options().with_draft(Draft::Draft7).build(&schema).unwrap();
        let instance = json!(["a", 1]);
        assert_eq!(
            errors_of(&validator, &instance),
            json!([
                {
                    "message": "Additional items are not allowed (1 was unexpected)",
                    "instancePath": [],
                    "schemaPath": ["additionalItems"],
                    "kind": {"type": "additionalItems", "limit": 1}
                }
            ])
        );
    }

    // `contentEncoding`/`contentMediaType` only assert in draft <= 7; draft 2019-09+ treats them
    // as annotation-only keywords that never produce a validation error.
    #[test_case(
        &json!({"contentEncoding": "base64"}),
        &json!("not base64!!!"),
        &json!([
            {
                "message": "\"not base64!!!\" is not compliant with \"base64\" content encoding",
                "instancePath": [],
                "schemaPath": ["contentEncoding"],
                "kind": {"type": "contentEncoding", "content_encoding": "base64"}
            }
        ])
    )]
    #[test_case(
        &json!({"contentMediaType": "application/json"}),
        &json!("not json"),
        &json!([
            {
                "message": "\"not json\" is not compliant with \"application/json\" media type",
                "instancePath": [],
                "schemaPath": ["contentMediaType"],
                "kind": {"type": "contentMediaType", "content_media_type": "application/json"}
            }
        ])
    )]
    fn content_keyword_maps_to_content_kind(schema: &Value, instance: &Value, expected: &Value) {
        let validator = options().with_draft(Draft::Draft7).build(schema).unwrap();
        assert_eq!(errors_of(&validator, instance), *expected);
    }

    #[test]
    fn utf8_decode_failure_after_content_encoding_maps_to_from_utf8_kind() {
        let schema = json!({"contentMediaType": "application/json", "contentEncoding": "base64"});
        let validator = options().with_draft(Draft::Draft7).build(&schema).unwrap();
        // Base64 for bytes 0xFF 0xFE, which is not valid UTF-8.
        let instance = json!("//4=");
        // `ValidationError::from_utf8` carries no instance/schema location, unlike other kinds.
        assert_eq!(
            errors_of(&validator, &instance),
            json!([
                {
                    "message": "invalid utf-8 sequence of 1 bytes from index 0",
                    "instancePath": [],
                    "schemaPath": [],
                    "kind": {"type": "fromUtf8", "message": "invalid utf-8 sequence of 1 bytes from index 0"}
                }
            ])
        );
    }

    struct RejectOddValidator;

    impl Keyword for RejectOddValidator {
        fn validate<'i>(&self, instance: &'i Value) -> Result<(), ValidationError<'i>> {
            if instance.as_u64().is_some_and(|n| n % 2 != 0) {
                return Err(ValidationError::custom("value must be even"));
            }
            Ok(())
        }

        fn is_valid(&self, instance: &Value) -> bool {
            instance.as_u64().is_none_or(|n| n % 2 == 0)
        }
    }

    #[test]
    fn custom_keyword_maps_to_custom_kind() {
        let schema = json!({"even-number": true, "type": "integer"});
        let validator = options()
            .with_keyword(
                "even-number",
                |_: &Map<String, Value>, _: &Value, _: Location| {
                    Ok(Box::new(RejectOddValidator) as Box<dyn Keyword>)
                },
            )
            .build(&schema)
            .unwrap();
        // Exercises `validate`/`is_valid`, which the `iter_errors`-based assertion below does not cover.
        assert!(validator.validate(&json!(4)).is_ok());
        assert!(validator.is_valid(&json!(4)));
        assert!(!validator.is_valid(&json!(3)));
        let instance = json!(3);
        assert_eq!(
            errors_of(&validator, &instance),
            json!([
                {
                    "message": "value must be even",
                    "instancePath": [],
                    "schemaPath": ["even-number"],
                    "kind": {"type": "custom", "keyword": "even-number", "message": "value must be even"}
                }
            ])
        );
    }

    #[test]
    fn backtrack_limit_exceeded_maps_to_backtrack_limit_exceeded_kind() {
        let schema = json!({"pattern": r"(?<=ab)c"});
        let validator = options()
            .with_pattern_options(PatternOptions::fancy_regex().backtrack_limit(1))
            .build(&schema)
            .unwrap();
        let instance = json!("abc");
        assert_eq!(
            errors_of(&validator, &instance),
            json!([
                {
                    "message": "Error executing regex: Max limit for backtracking count exceeded",
                    "instancePath": [],
                    "schemaPath": ["pattern"],
                    "kind": {
                        "type": "backtrackLimitExceeded",
                        "message": "Error executing regex: Max limit for backtracking count exceeded"
                    }
                }
            ])
        );
    }

    // `catch_unwind` is a no-op under `panic = "abort"` (wasm32-wasip1's default), so this
    // recovered-panic path only runs where the panic strategy is `unwind`.
    #[cfg(panic = "unwind")]
    #[test]
    fn regex_engine_panic_recovery_maps_to_regex_engine_failure_kind() {
        // Recovered `regex-automata` panic: https://github.com/rust-lang/regex/issues/1344.
        let schema = json!({"type": "string", "pattern": r"^.{0,404600}$"});
        let validator = options()
            .with_pattern_options(PatternOptions::fancy_regex().size_limit(1_000_000_000))
            .build(&schema)
            .unwrap();
        let instance = json!("");
        assert_eq!(
            errors_of(&validator, &instance),
            json!([
                {
                    "message": "Regex engine failed to evaluate pattern '^.{0,404600}$'",
                    "instancePath": [],
                    "schemaPath": ["pattern"],
                    "kind": {
                        "type": "regexEngineFailure",
                        "message": "Regex engine failed to evaluate pattern '^.{0,404600}$'"
                    }
                }
            ])
        );
    }
}
