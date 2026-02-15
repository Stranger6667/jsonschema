#[jsonschema::validator(schema = r#"{"type":"string","minimum":"x"}"#)]
struct InvalidMinimumUnderStringTypeValidator;

fn main() {}
