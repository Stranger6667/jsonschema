#[jsonschema::validator(
    schema = r#"{"dependentRequired":{"foo":["bar","bar"]}}"#,
    draft = referencing::Draft::Draft201909
)]
struct InvalidDependentRequiredDuplicatesValidator;

fn main() {}
