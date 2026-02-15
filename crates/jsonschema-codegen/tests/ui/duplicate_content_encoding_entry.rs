#[jsonschema::validator(
    schema = "{}",
    content_encodings = {
        "e" => { check = crate::c, convert = crate::d },
        "e" => { check = crate::c, convert = crate::d },
    }
)]
struct Validator;

fn main() {}
