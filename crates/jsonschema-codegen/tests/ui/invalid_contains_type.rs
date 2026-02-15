#[jsonschema::validator(schema = r#"{"contains":42}"#)]
struct InvalidContainsTypeValidator;

fn main() {}
