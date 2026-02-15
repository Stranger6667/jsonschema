#[jsonschema::validator(schema = r#"{"not":42}"#)]
struct InvalidNotTypeValidator;

fn main() {}
