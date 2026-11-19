#[jsonschema::validator(schema = r##"{"minItems":-1,"maxItems":3}"##)]
struct InvalidMinItemsBesideMaxItemsValidator;

fn main() {}
