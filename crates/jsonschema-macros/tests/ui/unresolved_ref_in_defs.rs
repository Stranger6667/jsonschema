#[jsonschema::validator(schema = r##"{"$defs":{"ok":{}},"$ref":"#/$defs/missing"}"##)]
struct Validator;

fn main() {}
