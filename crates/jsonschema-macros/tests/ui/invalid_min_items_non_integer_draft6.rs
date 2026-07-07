#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-06/schema#","type":"array","minItems":2.5}"#
)]
struct InvalidMinItemsNonIntegerDraft6Validator;

fn main() {}
