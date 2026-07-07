#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-06/schema#","exclusiveMinimum":true}"#
)]
struct InvalidExclusiveMinimumBooleanDraft6Validator;

fn main() {}
