#[jsonschema::validator(schema = r#"{"allOf":[]}"#)]
struct InvalidAllOfEmptyValidator;

fn main() {}
