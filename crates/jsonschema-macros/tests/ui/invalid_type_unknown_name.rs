#[jsonschema::validator(schema = r#"{"type":"bogus"}"#)]
struct Validator;

fn main() {}
