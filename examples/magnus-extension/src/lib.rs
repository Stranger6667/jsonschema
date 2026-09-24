use jsonschema::{
    json::{Magnus, RbNode},
    paths::Location,
    Keyword, ValidationError,
};
use magnus::{
    function, rb_sys::FromRawValue, value::Lazy, Error, ExceptionClass, Module, RString, Ruby,
    Value,
};
use serde_json::{Map, Value as Json};

#[allow(unsafe_code)]
fn ruby_value(node: RbNode<'_>) -> Value {
    unsafe { Value::from_raw(node.raw()) }
}

// `"uppercase": true` accepts strings without lowercase letters; other values are left alone.
struct Uppercase;

impl<'i> Keyword<'i, Magnus> for Uppercase {
    fn validate(&self, instance: RbNode<'i>) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom(format!(
                "{} is not uppercase",
                ruby_value(instance)
            )))
        }
    }

    fn is_valid(&self, instance: RbNode<'i>) -> bool {
        RString::from_value(ruby_value(instance)).is_none_or(|text| {
            text.to_string()
                .is_ok_and(|text| !text.chars().any(char::is_lowercase))
        })
    }
}

// The keyword factory signature returns a `Result`.
#[allow(clippy::unnecessary_wraps)]
fn uppercase<'a>(
    _: &'a Map<String, Json>,
    _: &'a Json,
    _: Location,
) -> Result<Box<dyn for<'i> Keyword<'i, Magnus>>, ValidationError<'a>> {
    Ok(Box::new(Uppercase))
}

#[jsonschema::validator(
    path = "schema.json",
    backend = Magnus,
    keywords = { "uppercase" => crate::uppercase }
)]
struct Event;

static ERROR: Lazy<ExceptionClass> = Lazy::new(|ruby| {
    ruby.define_module("Events")
        .expect("module")
        .define_error("Error", ruby.exception_standard_error())
        .expect("error class")
});

fn is_valid(instance: Value) -> Result<bool, Error> {
    Event::is_valid(&instance)
}

// Raises `Events::Error` with the first error.
fn validate(ruby: &Ruby, instance: Value) -> Result<(), Error> {
    match Event::validate(&instance)? {
        Ok(()) => Ok(()),
        Err(error) => Err(Error::new(ruby.get_inner(&ERROR), error.to_string())),
    }
}

fn errors(instance: Value) -> Result<Vec<String>, Error> {
    Ok(Event::iter_errors(&instance)?
        .map(|error| error.to_string())
        .collect())
}

#[magnus::init(name = "jsonschema_example_magnus")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Events")?;
    ruby.get_inner(&ERROR);
    module.define_module_function("valid?", function!(is_valid, 1))?;
    module.define_module_function("validate!", function!(validate, 1))?;
    module.define_module_function("errors", function!(errors, 1))?;
    Ok(())
}
