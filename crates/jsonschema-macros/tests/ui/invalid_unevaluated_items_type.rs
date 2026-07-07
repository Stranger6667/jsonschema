#[jsonschema::validator(
    schema = r#"{"unevaluatedItems":42}"#,
    draft = referencing::Draft::Draft202012
)]
struct InvalidUnevaluatedItemsTypeValidator;

fn main() {}
