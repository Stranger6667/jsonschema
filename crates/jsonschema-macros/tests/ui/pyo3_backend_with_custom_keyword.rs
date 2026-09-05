fn even_factory<'a>(
    _: &'a serde_json::Map<String, serde_json::Value>,
    _: &'a serde_json::Value,
    _: jsonschema::paths::Location,
) -> Result<Box<dyn jsonschema::Keyword<'a>>, jsonschema::ValidationError<'a>> {
    unimplemented!()
}

#[jsonschema::validator(schema = "{}", backend = Pyo3, keywords = { "even" => crate::even_factory })]
struct Validator;

fn main() {}
