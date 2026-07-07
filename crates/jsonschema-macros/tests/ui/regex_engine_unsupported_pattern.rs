#[jsonschema::validator(
    schema = r#"{"type":"string","pattern":"^(?!eo:)"}"#,
    pattern_options = {
        engine = regex,
    }
)]
struct Validator;

fn main() {}
