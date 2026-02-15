#[jsonschema::validator(schema = r#"{"items":[{}],"additionalItems":1}"#)]
struct InvalidAdditionalItemsTypeValidator;

fn main() {}
