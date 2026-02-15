#[jsonschema::validator(
    schema = "{}",
    resources = { "json-schema:///ext" => { schema = "{" } }
)]
struct Validator;

fn main() {}
