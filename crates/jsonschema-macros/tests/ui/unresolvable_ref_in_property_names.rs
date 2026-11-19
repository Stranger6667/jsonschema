#[jsonschema::validator(schema = r##"{"propertyNames":{"$ref":"#/$defs/missing"}}"##)]
struct UnresolvableRefInPropertyNamesValidator;

fn main() {}
