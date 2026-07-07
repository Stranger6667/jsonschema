#[jsonschema::validator(schema = r#"{"type":"array","maxItems":-1}"#)]
struct InvalidMaxItemsValueValidator;

fn main() {}
