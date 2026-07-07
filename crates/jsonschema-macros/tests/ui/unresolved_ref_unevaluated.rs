#[jsonschema::validator(schema = r##"{"$ref":"#/missing","unevaluatedProperties":false,"unevaluatedItems":false}"##)]
struct Validator;

fn main() {}
