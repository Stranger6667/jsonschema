#[jsonschema::validator(schema = r#"{"$ref":"http://example.com/remote"}"#)]
struct Validator;

fn main() {}
