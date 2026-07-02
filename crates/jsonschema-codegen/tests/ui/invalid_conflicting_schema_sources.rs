#[jsonschema::validator(path = "schema.json", schema = r#"{"type":"string"}"#)]
struct InvalidConflictingSchemaSources;

fn main() {}
