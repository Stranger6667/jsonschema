#[jsonschema::validator(
    schema = r#"{"type":"string","pattern":"[a-z]+"}"#,
    pattern_options = {
        engine = regex,
        size_limit = 1,
        dfa_size_limit = 1,
    }
)]
struct Validator;

fn main() {}
