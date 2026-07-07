#[jsonschema::validator(
    schema = r#"{"dependentSchemas":42}"#,
    draft = referencing::Draft::Draft201909
)]
struct InvalidDependentSchemasNotObjectValidator;

fn main() {}
