#[jsonschema::validator(
    schema = "{}",
    pattern_options = { size_limit = 999999999999999999999999999999 }
)]
struct Validator;

fn main() {}
