#[jsonschema::validator(schema = r#"{"type":"string","pattern":123}"#)]
struct NonStringPatternValueValidator;

fn main() {}
