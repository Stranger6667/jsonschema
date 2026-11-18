#[jsonschema::validator(
    schema = r#"{"properties": {"a": {"$dynamicAnchor": "x"}}}"#,
    methods = { evaluate = true }
)]
struct Validator;

fn main() {}
