#[jsonschema::validator(schema = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","unevaluatedProperties":false,"patternProperties":{"(":{}}}"#)]
struct Validator;

fn main() {}
