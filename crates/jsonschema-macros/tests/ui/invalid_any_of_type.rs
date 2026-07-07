#[jsonschema::validator(schema = r#"{"anyOf":"x"}"#)]
struct InvalidAnyOfTypeValidator;

fn main() {}
