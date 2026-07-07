#[jsonschema::validator(
    schema = r#"{"type":"string","format":"email"}"#,
    email_options = {
        unknown = true,
    }
)]
struct InvalidEmailOptionsKey;

fn main() {}
