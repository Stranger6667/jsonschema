#[jsonschema::validator(
    schema = r#"{"type":"string"}"#,
    draft = referencing::Draft::Draf202012
)]
struct InvalidDraftValueValidator;

fn main() {}
