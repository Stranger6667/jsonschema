#[jsonschema::validator(schema = r#"{"multipleOf":-1}"#)]
struct InvalidMultipleOfNegativeValidator;

fn main() {}
