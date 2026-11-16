#[jsonschema::validator(
    schema = r#"{"properties": {}}"#,
    methods = { evaluate = true }
)]
struct Validator;

fn main() {}
