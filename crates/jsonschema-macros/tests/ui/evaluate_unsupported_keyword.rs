#[jsonschema::validator(
    schema = r#"{"minimum": 1}"#,
    methods = { evaluate = true }
)]
struct Validator;

fn main() {}
