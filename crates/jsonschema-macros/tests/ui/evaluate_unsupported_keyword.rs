#[jsonschema::validator(
    schema = r#"{"properties": {"a": {"$schema": "http://json-schema.org/draft-07/schema#"}}}"#,
    methods = { evaluate = true }
)]
struct Validator;

fn main() {}
