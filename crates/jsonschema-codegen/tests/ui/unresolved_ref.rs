#[jsonschema::validator(schema = r##"{"$ref":"#/missing"}"##)]
struct Validator;

fn main() {}
