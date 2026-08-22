#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/custom","type":"string"}"#,
    draft = referencing::Draft::Draft202012,
    resources = {
        "json-schema:///meta/custom" => { schema = r#"{"$id":"json-schema:///meta/custom","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://example.com/vocab/made-up":true}}"# },
    }
)]
struct Validator;

fn main() {}
