#[jsonschema::validator(
    schema = "{}",
    content_encodings = { "e" => { check = crate::c } }
)]
struct Validator;

fn main() {}
