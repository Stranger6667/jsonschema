#[jsonschema::validator(
    schema = "{}",
    pattern_options = { engine = bogus }
)]
struct Validator;

fn main() {}
