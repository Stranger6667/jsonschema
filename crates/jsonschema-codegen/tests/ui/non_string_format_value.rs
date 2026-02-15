#[jsonschema::validator(
    schema = r#"{"type":"string","format":123}"#,
    validate_formats = true
)]
struct NonStringFormatValidator;

fn main() {}
