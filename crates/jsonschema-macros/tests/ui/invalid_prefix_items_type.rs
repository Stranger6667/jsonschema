#[jsonschema::validator(schema = r#"{"prefixItems":42}"#)]
struct InvalidPrefixItemsTypeValidator;

fn main() {}
