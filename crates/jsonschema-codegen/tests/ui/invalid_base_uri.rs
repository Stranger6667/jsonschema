#[jsonschema::validator(
    schema = r#"{"type":"integer"}"#,
    base_uri = "http://exa mple.com"
)]
struct InvalidBaseUri;

fn main() {}
