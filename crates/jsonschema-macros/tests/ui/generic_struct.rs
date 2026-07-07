#[jsonschema::validator(schema = r#"{"type":"string"}"#)]
struct Validator<T>(std::marker::PhantomData<T>);

fn main() {}
