#[jsonschema::validator(schema = r#"{"type":"object","required":["a","a"]}"#)]
struct InvalidRequiredDuplicatesValidator;

fn main() {}
