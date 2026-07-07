#[jsonschema::validator(schema = r#"{"type":"string"}"#, schema = r#"{"type":"number"}"#)]
struct Validator;

fn main() {}
