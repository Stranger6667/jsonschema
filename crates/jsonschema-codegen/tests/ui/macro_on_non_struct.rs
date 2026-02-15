#[jsonschema::validator(schema = r#"{"type":"string"}"#)]
fn not_a_struct() {}

fn main() {}
