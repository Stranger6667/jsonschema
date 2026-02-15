#[jsonschema::validator(schema = r#"{"type":"array","contains":{},"maxContains":-1}"#)]
struct Validator;

fn main() {}
