#[jsonschema::validator(schema = r#"{"type":"array","contains":{},"minContains":"x"}"#)]
struct Validator;

fn main() {}
