#[jsonschema::validator(
    schema = r#"{"type": "string"}"#,
    methods = { evaluate = true }
)]
struct Validator;

fn main() {}
