#[jsonschema::validator(schema = r#"{"additionalProperties":42}"#)]
struct InvalidAdditionalPropertiesTypeValidator;

fn main() {}
