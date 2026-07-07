#[jsonschema::validator(
    schema = r#"{"dependentSchemas":{"foo":42}}"#,
    draft = referencing::Draft::Draft201909
)]
struct InvalidDependentSchemasTypeValidator;

fn main() {}
