#[jsonschema::validator(schema = r#"{"minimum":"x"}"#)]
struct InvalidMinimumTypeValidator;

fn main() {}
