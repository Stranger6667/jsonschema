#[jsonschema::validator(
    schema = r#"{"properties": {"a": {"unevaluatedItems": false}}}"#,
    methods = { evaluate = true }
)]
struct Validator;

fn main() {}
