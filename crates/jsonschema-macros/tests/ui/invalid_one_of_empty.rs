#[jsonschema::validator(schema = r#"{"oneOf":[]}"#)]
struct InvalidOneOfEmptyValidator;

fn main() {}
