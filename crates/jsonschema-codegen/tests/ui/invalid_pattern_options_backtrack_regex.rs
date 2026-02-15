#[jsonschema::validator(
    schema = r#"{"type":"string"}"#,
    pattern_options = {
        engine = regex,
        backtrack_limit = 10,
    }
)]
struct Validator;

fn main() {}
