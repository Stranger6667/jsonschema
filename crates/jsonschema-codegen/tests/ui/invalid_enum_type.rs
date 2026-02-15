#[jsonschema::validator(schema = r#"{"enum":1}"#)]
struct InvalidEnumTypeValidator;

fn main() {}
