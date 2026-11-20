#[jsonschema::validator(
    schema = r#"{"contentMediaType":"text/x-unknown","contentEncoding":42}"#,
    draft = referencing::Draft::Draft7
)]
struct ContentEncodingNonStringUnknownMediaTypeValidator;

fn main() {}
