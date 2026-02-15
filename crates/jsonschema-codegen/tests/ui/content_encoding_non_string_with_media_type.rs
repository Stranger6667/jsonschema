fn noop(_value: &str) -> bool {
    true
}

#[jsonschema::validator(
    schema = r#"{"$schema":"http://json-schema.org/draft-07/schema#","contentMediaType":"application/json","contentEncoding":42}"#,
    content_media_types = { "application/json" => crate::noop }
)]
struct ContentEncodingNonStringValidator;

fn main() {}
