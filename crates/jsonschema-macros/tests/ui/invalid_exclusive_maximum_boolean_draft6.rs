#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-06/schema#","exclusiveMaximum":true}"#
)]
struct InvalidExclusiveMaximumBooleanDraft6Validator;

fn main() {}
