#[jsonschema::validator(schema = r#"{"items":1}"#)]
struct InvalidItemsTypeValidator;

fn main() {}
