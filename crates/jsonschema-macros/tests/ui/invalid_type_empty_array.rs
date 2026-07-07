#[jsonschema::validator(schema = r#"{"type":[]}"#)]
struct InvalidTypeEmptyArrayValidator;

fn main() {}
