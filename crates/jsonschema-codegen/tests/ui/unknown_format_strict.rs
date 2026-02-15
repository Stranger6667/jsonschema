#[jsonschema::validator(
    schema = r#"{"type":"string","format":"made-up"}"#,
    validate_formats = true,
    ignore_unknown_formats = false
)]
struct UnknownFormatStrictValidator;

fn main() {}
