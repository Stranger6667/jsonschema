#[jsonschema::validator(schema = r#"{"type":"integer"}"#, draft = Draft4, draft = Draft7)]
struct Validator;

fn main() {}
