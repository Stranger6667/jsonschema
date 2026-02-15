#[jsonschema::validator(
    schema = r#"{"contentMediaType":42}"#,
    draft = referencing::Draft::Draft7
)]
struct InvalidContentMediaTypeTypeValidator;

fn main() {}
