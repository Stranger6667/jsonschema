#[jsonschema::validator(
    schema = "{}",
    resources = { "json-schema:///u" => { bogus = "x" } }
)]
struct Validator;

fn main() {}
