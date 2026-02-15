#[jsonschema::validator(schema = r#"{"type":["string",42]}"#)]
struct Validator;

fn main() {}
