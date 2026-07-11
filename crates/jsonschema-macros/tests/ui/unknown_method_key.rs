#[jsonschema::validator(
    schema = r#"{"type":"string"}"#,
    methods(bogus = false)
)]
struct UnknownMethodKeyValidator;

fn main() {}
