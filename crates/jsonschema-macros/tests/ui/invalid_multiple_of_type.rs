#[jsonschema::validator(schema = r#"{"multipleOf":"x"}"#)]
struct InvalidMultipleOfTypeValidator;

fn main() {}
