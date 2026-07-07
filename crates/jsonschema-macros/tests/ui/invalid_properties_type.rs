#[jsonschema::validator(schema = r#"{"properties": 1}"#)]
struct InvalidPropertiesTypeValidator;

fn main() {}
