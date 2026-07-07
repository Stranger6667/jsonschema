#[jsonschema::validator(
    schema = "{}",
    pattern_options = { size_limit = 1, size_limit = 2 }
)]
struct Validator;

fn main() {}
