#[jsonschema::validator(schema = r#"{"patternProperties":{"(":{}}}"#)]
struct Validator;

fn main() {}
