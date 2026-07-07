#[jsonschema::validator(
    schema = "{}",
    email_options = { required_tld = true, required_tld = false }
)]
struct Validator;

fn main() {}
