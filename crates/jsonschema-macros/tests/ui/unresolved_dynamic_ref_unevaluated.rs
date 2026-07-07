#[jsonschema::validator(schema = r##"{"$dynamicRef":"#/missing","unevaluatedProperties":false,"unevaluatedItems":false}"##)]
struct Validator;

fn main() {}
