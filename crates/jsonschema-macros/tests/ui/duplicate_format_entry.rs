fn currency_a(_value: &str) -> bool {
    true
}

fn currency_b(_value: &str) -> bool {
    true
}

#[jsonschema::validator(
    schema = r#"{"type":"string"}"#,
    formats = {
        "currency" => crate::currency_a,
        "currency" => crate::currency_b,
    }
)]
struct DuplicateFormatEntryValidator;

fn main() {}
