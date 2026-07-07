#[jsonschema::validator(
    schema = r#"{"dependentRequired":42}"#,
    draft = referencing::Draft::Draft201909
)]
struct InvalidDependentRequiredTypeValidator;

fn main() {}
