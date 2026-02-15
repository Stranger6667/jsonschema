#![no_main]

use libfuzzer_sys::fuzz_target;
use referencing::Draft;
use serde_json::Value;
use std::sync::LazyLock;

const SCHEMA_DRAFT7_OBJECT: &str = r#"{"type":"object","properties":{"name":{"type":"string"},"age":{"type":"integer","minimum":0}},"required":["name"],"additionalProperties":false}"#;
const SCHEMA_DRAFT7_MIXED_TYPES: &str = r#"{"type":["string","integer"],"minLength":2}"#;
const SCHEMA_DRAFT7_ARRAY: &str =
    r#"{"type":"array","items":{"type":"integer"},"minItems":1,"maxItems":5,"uniqueItems":true}"#;
const SCHEMA_DRAFT201909_REF_SIBLINGS: &str =
    r##"{"$defs":{"num":{"type":"number"}},"$ref":"#/$defs/num","type":"string"}"##;
const SCHEMA_DRAFT202012_PREFIX_ITEMS: &str =
    r#"{"type":"array","prefixItems":[{"type":"integer"},{"type":"string"}],"items":false}"#;
const SCHEMA_DRAFT201909_DEPENDENT_SCHEMAS: &str =
    r#"{"type":"object","dependentSchemas":{"x":{"required":["y"]}}}"#;

#[jsonschema::validator(
    schema = r#"{"type":"object","properties":{"name":{"type":"string"},"age":{"type":"integer","minimum":0}},"required":["name"],"additionalProperties":false}"#,
    draft = referencing::Draft::Draft7
)]
struct Draft7ObjectValidator;

#[jsonschema::validator(
    schema = r#"{"type":["string","integer"],"minLength":2}"#,
    draft = referencing::Draft::Draft7
)]
struct Draft7MixedTypesValidator;

#[jsonschema::validator(
    schema = r#"{"type":"array","items":{"type":"integer"},"minItems":1,"maxItems":5,"uniqueItems":true}"#,
    draft = referencing::Draft::Draft7
)]
struct Draft7ArrayValidator;

#[jsonschema::validator(
    schema = r##"{"$defs":{"num":{"type":"number"}},"$ref":"#/$defs/num","type":"string"}"##,
    draft = referencing::Draft::Draft201909
)]
struct Draft201909RefSiblingsValidator;

#[jsonschema::validator(
    schema = r#"{"type":"array","prefixItems":[{"type":"integer"},{"type":"string"}],"items":false}"#,
    draft = referencing::Draft::Draft202012
)]
struct Draft202012PrefixItemsValidator;

#[jsonschema::validator(
    schema = r#"{"type":"object","dependentSchemas":{"x":{"required":["y"]}}}"#,
    draft = referencing::Draft::Draft201909
)]
struct Draft201909DependentSchemasValidator;

static DYNAMIC_DRAFT7_OBJECT: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| build_dynamic(SCHEMA_DRAFT7_OBJECT, Draft::Draft7));
static DYNAMIC_DRAFT7_MIXED_TYPES: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| build_dynamic(SCHEMA_DRAFT7_MIXED_TYPES, Draft::Draft7));
static DYNAMIC_DRAFT7_ARRAY: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| build_dynamic(SCHEMA_DRAFT7_ARRAY, Draft::Draft7));
static DYNAMIC_DRAFT201909_REF_SIBLINGS: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| build_dynamic(SCHEMA_DRAFT201909_REF_SIBLINGS, Draft::Draft201909));
static DYNAMIC_DRAFT202012_PREFIX_ITEMS: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| build_dynamic(SCHEMA_DRAFT202012_PREFIX_ITEMS, Draft::Draft202012));
static DYNAMIC_DRAFT201909_DEPENDENT_SCHEMAS: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| build_dynamic(SCHEMA_DRAFT201909_DEPENDENT_SCHEMAS, Draft::Draft201909));

struct ValidatorPair {
    name: &'static str,
    codegen_is_valid: fn(&Value) -> bool,
    dynamic: &'static LazyLock<jsonschema::Validator>,
}

static VALIDATOR_PAIRS: &[ValidatorPair] = &[
    ValidatorPair {
        name: "draft7_object",
        codegen_is_valid: Draft7ObjectValidator::is_valid,
        dynamic: &DYNAMIC_DRAFT7_OBJECT,
    },
    ValidatorPair {
        name: "draft7_mixed_types",
        codegen_is_valid: Draft7MixedTypesValidator::is_valid,
        dynamic: &DYNAMIC_DRAFT7_MIXED_TYPES,
    },
    ValidatorPair {
        name: "draft7_array",
        codegen_is_valid: Draft7ArrayValidator::is_valid,
        dynamic: &DYNAMIC_DRAFT7_ARRAY,
    },
    ValidatorPair {
        name: "draft2019_09_ref_siblings",
        codegen_is_valid: Draft201909RefSiblingsValidator::is_valid,
        dynamic: &DYNAMIC_DRAFT201909_REF_SIBLINGS,
    },
    ValidatorPair {
        name: "draft2020_12_prefix_items",
        codegen_is_valid: Draft202012PrefixItemsValidator::is_valid,
        dynamic: &DYNAMIC_DRAFT202012_PREFIX_ITEMS,
    },
    ValidatorPair {
        name: "draft2019_09_dependent_schemas",
        codegen_is_valid: Draft201909DependentSchemasValidator::is_valid,
        dynamic: &DYNAMIC_DRAFT201909_DEPENDENT_SCHEMAS,
    },
];

fn build_dynamic(schema: &str, draft: Draft) -> jsonschema::Validator {
    let value: Value = serde_json::from_str(schema).expect("schema literals are valid JSON");
    jsonschema::options()
        .with_draft(draft)
        .build(&value)
        .expect("schema literals should build dynamically")
}

fuzz_target!(|data: &[u8]| {
    let Ok(instance) = serde_json::from_slice::<Value>(data) else {
        return;
    };

    for pair in VALIDATOR_PAIRS {
        let codegen = (pair.codegen_is_valid)(&instance);
        let dynamic = pair.dynamic.is_valid(&instance);
        assert_eq!(
            codegen, dynamic,
            "is_valid mismatch for schema `{}` on instance {instance}",
            pair.name
        );
    }
});
