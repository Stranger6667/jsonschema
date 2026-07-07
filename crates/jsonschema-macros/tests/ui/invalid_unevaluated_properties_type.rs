#[jsonschema::validator(
    schema = r#"{"unevaluatedProperties":42}"#,
    draft = referencing::Draft::Draft202012
)]
struct InvalidUnevaluatedPropertiesTypeValidator;

fn main() {}
