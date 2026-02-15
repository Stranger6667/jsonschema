#[jsonschema::validator(schema = r#"{"type":"object","minProperties":"x"}"#)]
struct Validator;

fn main() {}
