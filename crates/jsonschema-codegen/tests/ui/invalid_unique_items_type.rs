#[jsonschema::validator(schema = r#"{"uniqueItems":42}"#)]
struct InvalidUniqueItemsTypeValidator;

fn main() {}
