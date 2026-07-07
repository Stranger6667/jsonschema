#[jsonschema::validator(schema = r#"{"allOf":"x"}"#)]
struct InvalidAllOfTypeValidator;

fn main() {}
