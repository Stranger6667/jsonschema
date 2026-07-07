#[jsonschema::validator(
    schema = r#"{"type":"string"}"#,
    pattern_options = {
        engine = fancy_regex,
        unknown = 1,
    }
)]
struct Validator;

fn main() {}
