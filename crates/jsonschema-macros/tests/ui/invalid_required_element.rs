#[jsonschema::validator(schema = r#"{"required":[42]}"#)]
struct Validator;

fn main() {}
