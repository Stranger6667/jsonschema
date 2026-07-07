#[jsonschema::validator(schema = r##"{"$schema":"https://json-schema.org/draft/2019-09/schema","$recursiveRef":"#/missing","unevaluatedProperties":false,"unevaluatedItems":false}"##)]
struct Validator;

fn main() {}
