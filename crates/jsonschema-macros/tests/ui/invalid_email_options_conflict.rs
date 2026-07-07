#[jsonschema::validator(
    schema = r#"{"type":"string","format":"email"}"#,
    email_options = {
        minimum_sub_domains = 1,
        required_tld = true,
    }
)]
struct InvalidEmailOptionsConflict;

fn main() {}
