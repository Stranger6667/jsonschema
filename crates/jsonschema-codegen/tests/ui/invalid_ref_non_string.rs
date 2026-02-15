#[jsonschema::validator(schema = r#"{"$ref":42}"#)]
struct Validator;

fn main() {}
