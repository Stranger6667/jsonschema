#[jsonschema::validator(
    schema = r#"{"contentEncoding":42}"#,
    draft = referencing::Draft::Draft7
)]
struct InvalidContentEncodingTypeValidator;

fn main() {}
