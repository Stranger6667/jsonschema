#[jsonschema::validator(schema = r#"{"multipleOf":0}"#)]
struct InvalidMultipleOfNonPositiveValidator;

fn main() {}
