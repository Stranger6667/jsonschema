#![no_main]

use libfuzzer_sys::fuzz_target;
use referencing::Draft;
use serde_json::Value;
use std::sync::LazyLock;

struct Case {
    name: &'static str,
    codegen_is_valid: fn(&Value) -> bool,
    codegen_validate: for<'a> fn(&'a Value) -> Result<(), jsonschema::ValidationError<'a>>,
    runtime: jsonschema::Validator,
}

fn build_runtime(schema: &str, draft: Draft) -> jsonschema::Validator {
    let value: Value = serde_json::from_str(schema).expect("schema literal is valid JSON");
    jsonschema::options()
        .with_draft(draft)
        .build(&value)
        .expect("schema literal builds at run time")
}

macro_rules! cases {
    ($( $name:ident => $draft:ident, $schema:literal ; )+) => {
        $(
            #[jsonschema::validator(schema = $schema, draft = $draft)]
            struct $name;
        )+

        fn build_cases() -> Vec<Case> {
            vec![ $(
                Case {
                    name: stringify!($name),
                    codegen_is_valid: $name::is_valid,
                    codegen_validate: $name::validate,
                    runtime: build_runtime($schema, Draft::$draft),
                },
            )+ ]
        }
    };
}

cases! {
    Object => Draft7,
        r#"{"type":"object","properties":{"name":{"type":"string"},"age":{"type":"integer","minimum":0}},"required":["name"],"additionalProperties":false}"#;
    ObjectPatterns => Draft7,
        r#"{"type":"object","properties":{"a":{"type":"integer"}},"patternProperties":{"^x_":{"type":"string"}},"additionalProperties":false}"#;
    PropertyNames => Draft202012,
        r#"{"type":"object","propertyNames":{"minLength":2},"minProperties":1,"maxProperties":3}"#;
    MixedTypes => Draft7,
        r#"{"type":["string","integer"],"minLength":2}"#;
    EnumConst => Draft7,
        r#"{"type":"object","properties":{"color":{"enum":["red","green","blue"]},"version":{"const":"1.0"}},"required":["color","version"]}"#;
    StringBounds => Draft7,
        r#"{"type":"string","minLength":2,"maxLength":5,"pattern":"^a"}"#;
    PatternExact => Draft7,
        r#"{"type":"string","pattern":"^abc$"}"#;
    PatternAlternation => Draft7,
        r#"{"type":"string","pattern":"^(cat|dog)$"}"#;
    Numeric => Draft7,
        r#"{"type":"integer","minimum":-5,"exclusiveMaximum":10,"multipleOf":3}"#;
    NumberFractionalMultiple => Draft7,
        r#"{"type":"number","multipleOf":0.25}"#;
    Not => Draft7,
        r#"{"not":{"type":"string"}}"#;
    Array => Draft7,
        r#"{"type":"array","items":{"type":"integer"},"minItems":1,"maxItems":5,"uniqueItems":true}"#;
    Contains => Draft202012,
        r#"{"type":"array","contains":{"type":"integer"},"minContains":1,"maxContains":2}"#;
    PrefixItems => Draft202012,
        r#"{"type":"array","prefixItems":[{"type":"integer"},{"type":"string"}],"items":false}"#;
    UnevaluatedProps => Draft202012,
        r#"{"type":"object","properties":{"a":{"type":"integer"}},"unevaluatedProperties":false}"#;
    UnevaluatedItems => Draft202012,
        r#"{"type":"array","prefixItems":[{"type":"integer"}],"unevaluatedItems":false}"#;
    DependentSchemas => Draft201909,
        r#"{"type":"object","dependentSchemas":{"x":{"required":["y"]}}}"#;
    AnyOf => Draft7,
        r#"{"anyOf":[{"type":"string","minLength":3},{"type":"integer"}]}"#;
    OneOfDiscriminator => Draft202012,
        r#"{"oneOf":[{"type":"object","properties":{"k":{"const":1}},"required":["k"]},{"type":"object","properties":{"k":{"const":2}},"required":["k"]}]}"#;
    RefSiblings => Draft201909,
        r##"{"$defs":{"num":{"type":"number"}},"$ref":"#/$defs/num","type":"string"}"##;
    RefLocal => Draft7,
        r##"{"definitions":{"pos":{"type":"integer","minimum":1}},"type":"object","properties":{"id":{"$ref":"#/definitions/pos"}},"required":["id"]}"##;
}

static CASES: LazyLock<Vec<Case>> = LazyLock::new(build_cases);

fuzz_target!(|data: &[u8]| {
    let Ok(instance) = serde_json::from_slice::<Value>(data) else {
        return;
    };

    for case in CASES.iter() {
        let codegen = (case.codegen_is_valid)(&instance);
        let runtime = case.runtime.is_valid(&instance);
        assert_eq!(
            codegen, runtime,
            "is_valid mismatch [{}] on {instance}",
            case.name
        );

        let codegen_validate = (case.codegen_validate)(&instance);
        let runtime_validate = case.runtime.validate(&instance);
        assert_eq!(
            codegen_validate.is_ok(),
            runtime_validate.is_ok(),
            "validate ok/err mismatch [{}] on {instance}",
            case.name
        );

        if let (Err(codegen_error), Err(runtime_error)) = (&codegen_validate, &runtime_validate) {
            assert_eq!(
                codegen_error.instance_path(),
                runtime_error.instance_path(),
                "instance_path mismatch [{}] on {instance}",
                case.name
            );
            assert_eq!(
                codegen_error.to_string(),
                runtime_error.to_string(),
                "message mismatch [{}] on {instance}",
                case.name
            );
            assert_eq!(
                codegen_error.schema_path(),
                runtime_error.schema_path(),
                "schema_path mismatch [{}] on {instance}",
                case.name
            );
        }
    }
});
