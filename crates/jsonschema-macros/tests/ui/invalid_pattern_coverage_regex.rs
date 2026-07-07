#[jsonschema::validator(schema = r#"{"patternProperties":{"(":{}},"additionalProperties":false}"#)]
struct Validator;

fn main() {}
