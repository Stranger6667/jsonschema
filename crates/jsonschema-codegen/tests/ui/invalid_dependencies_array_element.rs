#[jsonschema::validator(
    schema = r#"{"dependencies":{"foo":[42]}}"#,
    draft = referencing::Draft::Draft7
)]
struct InvalidDependenciesArrayElementValidator;

fn main() {}
