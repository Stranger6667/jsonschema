#[jsonschema::validator(
    schema = r#"{"dependentRequired":{"foo":[42]}}"#,
    draft = referencing::Draft::Draft201909
)]
struct InvalidDependentRequiredElementValidator;

fn main() {}
