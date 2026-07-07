#[jsonschema::validator(
    schema = "{}",
    content_encodings = { "e" => { bogus = crate::c } }
)]
struct Validator;

fn main() {}
