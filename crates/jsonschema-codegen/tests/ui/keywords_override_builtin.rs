fn factory<'a>(
    _parent: &'a serde_json::Map<String, serde_json::Value>,
    _value: &'a serde_json::Value,
    _path: jsonschema::paths::Location,
) -> Result<Box<dyn jsonschema::Keyword>, jsonschema::ValidationError<'a>> {
    unimplemented!()
}

#[jsonschema::validator(
    schema = r#"{"type":"integer"}"#,
    keywords = { "type" => crate::factory }
)]
struct Validator;

fn main() {}
