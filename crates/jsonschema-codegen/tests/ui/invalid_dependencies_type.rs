#[jsonschema::validator(
    schema = r#"{"dependencies":"x"}"#,
    draft = referencing::Draft::Draft7
)]
struct InvalidDependenciesTypeValidator;

fn main() {}
