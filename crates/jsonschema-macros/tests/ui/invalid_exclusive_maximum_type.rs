#[jsonschema::validator(schema = r#"{"exclusiveMaximum":"x"}"#)]
struct InvalidExclusiveMaximumTypeValidator;

fn main() {}
