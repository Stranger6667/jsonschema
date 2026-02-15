#[jsonschema::validator(
    path = "../benchmark/data/recursive_schema.json",
    draft = referencing::Draft::Draft7
)]
struct InlineValidator;
