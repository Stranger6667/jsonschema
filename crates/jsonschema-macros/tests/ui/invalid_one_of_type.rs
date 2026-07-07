#[jsonschema::validator(schema = r#"{"oneOf":"x"}"#)]
struct InvalidOneOfTypeValidator;

fn main() {}
