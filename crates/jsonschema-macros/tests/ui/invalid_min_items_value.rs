#[jsonschema::validator(schema = r#"{"type":"array","minItems":"x"}"#)]
struct InvalidMinItemsValueValidator;

fn main() {}
