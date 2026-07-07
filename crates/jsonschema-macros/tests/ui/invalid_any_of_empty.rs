#[jsonschema::validator(schema = r#"{"anyOf":[]}"#)]
struct InvalidAnyOfEmptyValidator;

fn main() {}
