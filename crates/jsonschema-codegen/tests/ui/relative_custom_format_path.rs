fn custom_format(_: &str) -> bool {
    true
}

#[jsonschema::validator(
    schema = r#"{"type":"string","format":"custom"}"#,
    validate_formats = true,
    formats = {
        "custom" => custom_format
    }
)]
struct RelativeCustomFormatPathValidator;

fn main() {}
