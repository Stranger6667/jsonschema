#[jsonschema::validator(schema = r##"{"properties":{"a":{}},"additionalProperties":42}"##)]
struct InvalidAdditionalPropertiesTypeBesidePropertiesValidator;

fn main() {}
