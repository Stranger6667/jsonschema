#[jsonschema::validator(schema = r#"{"exclusiveMinimum":"x"}"#)]
struct InvalidExclusiveMinimumTypeValidator;

fn main() {}
