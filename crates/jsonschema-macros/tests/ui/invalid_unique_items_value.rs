#[jsonschema::validator(schema = r#"{"type":"array","uniqueItems":"yes"}"#)]
struct Validator;

fn main() {}
