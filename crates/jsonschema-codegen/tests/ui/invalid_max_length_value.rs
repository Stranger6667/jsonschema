#[jsonschema::validator(schema = r#"{"type":"string","maxLength":-1}"#)]
struct Validator;

fn main() {}
