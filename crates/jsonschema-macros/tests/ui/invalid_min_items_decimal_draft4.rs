#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-04/schema#","type":"array","minItems":2.0}"#
)]
struct InvalidMinItemsDecimalDraft4Validator;

fn main() {}
