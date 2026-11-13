#[jsonschema::validator(
    schema = "{}",
    pattern_options = { syntax = bogus }
)]
struct Validator;

fn main() {}
