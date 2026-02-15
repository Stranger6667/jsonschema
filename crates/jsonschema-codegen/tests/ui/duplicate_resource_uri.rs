#[jsonschema::validator(
    schema = r#"{"$ref": "json-schema:///a"}"#,
    resources = {
        "json-schema:///a" => { schema = r#"{"type":"string"}"# },
        "json-schema:///a" => { schema = r#"{"type":"integer"}"# },
    }
)]
struct Validator;

fn main() {}
