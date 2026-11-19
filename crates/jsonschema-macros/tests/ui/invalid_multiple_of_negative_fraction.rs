#[jsonschema::validator(schema = r##"{"multipleOf":-0.5}"##)]
struct InvalidMultipleOfNegativeFractionValidator;

fn main() {}
