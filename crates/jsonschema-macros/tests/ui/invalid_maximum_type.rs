#[jsonschema::validator(schema = r#"{"maximum":"x"}"#)]
struct InvalidMaximumTypeValidator;

fn main() {}
