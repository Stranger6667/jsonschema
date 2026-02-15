#[jsonschema::validator(schema = r#"{"propertyNames":42}"#)]
struct InvalidPropertyNamesTypeValidator;

fn main() {}
