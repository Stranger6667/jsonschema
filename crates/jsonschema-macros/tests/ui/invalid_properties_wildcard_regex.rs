#[jsonschema::validator(schema = r#"{"properties":{"a":{}},"patternProperties":{"(":{}},"additionalProperties":false}"#)]
struct Validator;

fn main() {}
