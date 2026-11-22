fn even(_: &serde_json::Number) -> bool {
    true
}

#[jsonschema::validator(
    schema = r#"{"format":"even"}"#,
    validate_formats = true,
    formats = {
        "even" => length(crate::even)
    }
)]
struct UnknownFormatWrapperValidator;

fn main() {}
