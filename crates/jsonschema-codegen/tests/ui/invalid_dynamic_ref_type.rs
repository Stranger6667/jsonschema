#[jsonschema::validator(
    schema = r#"{"$dynamicRef": 1}"#,
    draft = referencing::Draft::Draft202012
)]
struct InvalidDynamicRefTypeValidator;

fn main() {}
