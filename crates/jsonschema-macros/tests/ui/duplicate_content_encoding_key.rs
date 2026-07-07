#[jsonschema::validator(
    schema = "{}",
    content_encodings = { "e" => { check = crate::c, check = crate::d } }
)]
struct Validator;

fn main() {}
