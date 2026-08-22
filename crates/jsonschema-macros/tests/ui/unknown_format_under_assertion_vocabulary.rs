#[jsonschema::validator(
    schema = r#"{"$schema":"json-schema:///meta/format-assertion","format":"totally-made-up"}"#,
    draft = referencing::Draft::Draft202012,
    ignore_unknown_formats = true,
    resources = {
        "json-schema:///meta/format-assertion" => { schema = r#"{"$id":"json-schema:///meta/format-assertion","$schema":"https://json-schema.org/draft/2020-12/schema","$vocabulary":{"https://json-schema.org/draft/2020-12/vocab/core":true,"https://json-schema.org/draft/2020-12/vocab/format-assertion":true}}"# },
    }
)]
struct Validator;

fn main() {}
