//! `JSONSchema::JsonTextValidator`: validates instances given as JSON text.
use std::panic::AssertUnwindSafe;

use jsonschema::json::{Array, JsonNumber, JsonText, JsonTextDocument, JsonTextNode, Node, Object};
use magnus::{
    function, method, prelude::*, scan_args::scan_args, DataTypeFunctions, Error, RModule, RString,
    Ruby, Value,
};

use crate::{
    build_parsed_options, build_validator, catch_unwind_silent, handle_callback_panic,
    options::{extract_kwargs, CallbackRoots, CompilationRootsRef},
    raise_validation_error,
    ser::to_schema_value,
    to_ruby_error_value, BuiltValidator, Evaluation, InstanceRoot, KeywordCauseScope,
};

/// A validator for instances given as JSON text, without building Ruby objects for them.
///
/// Holds the same GC-protection state as `Validator`.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::JsonTextValidator", free_immediately, size, mark)]
pub struct JsonTextValidator {
    validator: jsonschema::Validator<JsonText>,
    mask: Option<String>,
    callback_roots: CallbackRoots,
    _compilation_roots: CompilationRootsRef,
}

impl DataTypeFunctions for JsonTextValidator {
    fn mark(&self, marker: &magnus::gc::Marker) {
        // Avoid panicking in Ruby GC mark paths; preserving existing roots is safer than aborting.
        let roots = match self.callback_roots.lock() {
            Ok(roots) => roots,
            Err(poisoned) => poisoned.into_inner(),
        };
        for root in roots.iter().copied() {
            marker.mark(root);
        }
    }
}

// Copied so Ruby code that validation runs cannot change the text under it.
fn json_text(ruby: &Ruby, instance: Value) -> Result<Vec<u8>, Error> {
    let Some(mut string) = RString::from_value(instance) else {
        return Err(Error::new(
            ruby.exception_type_error(),
            format!(
                "JSON text must be a String, not {}",
                instance.class().inspect()
            ),
        ));
    };
    // As `JSON.parse` does: binary strings are read as UTF-8, other encodings converted to it.
    let encoding = string.enc_get();
    if encoding != ruby.utf8_encindex()
        && encoding != ruby.usascii_encindex()
        && encoding != ruby.ascii8bit_encindex()
    {
        string = string.funcall("encode", ("UTF-8",))?;
    }
    // SAFETY: the bytes are copied before any Ruby code runs.
    #[allow(unsafe_code)]
    let bytes = unsafe { string.as_slice() }.to_vec();
    Ok(bytes)
}

fn parse<'a>(ruby: &Ruby, text: &'a [u8]) -> Result<JsonTextDocument<'a>, Error> {
    JsonText::parse(text)
        .map_err(|error| Error::new(ruby.exception_arg_error(), format!("Invalid JSON: {error}")))
}

/// What `JSON.parse` builds: members in document order, `Integer` for integers, `Float` otherwise.
pub(crate) fn text_node_to_ruby(ruby: &Ruby, node: &JsonTextNode<'_>) -> Result<Value, Error> {
    if let Some(object) = node.as_object() {
        let hash = ruby.hash_new();
        for (name, value) in object.members() {
            hash.aset(ruby.str_new(&name), text_node_to_ruby(ruby, &value)?)?;
        }
        return Ok(hash.as_value());
    }
    if let Some(array) = node.as_array() {
        let list = ruby.ary_new_capa(array.len());
        for element in array.elements() {
            list.push(text_node_to_ruby(ruby, &element)?)?;
        }
        return Ok(list.as_value());
    }
    if let Some(string) = node.as_string() {
        return Ok(ruby.str_new(&string).as_value());
    }
    if let Some(number) = node.as_number() {
        if let Some(value) = number.as_i64() {
            return Ok(ruby.integer_from_i64(value).as_value());
        }
        let digits = JsonNumber::as_str(&number);
        if digits.contains(['.', 'e', 'E']) {
            // Past `f64` this is infinite, as with `JSON.parse`.
            let value: f64 = digits.parse().expect("a checked number");
            return Ok(ruby.float_from_f64(value).as_value());
        }
        return ruby.str_new(&digits).funcall("to_i", ());
    }
    Ok(match node.as_boolean() {
        Some(true) => ruby.qtrue().as_value(),
        Some(false) => ruby.qfalse().as_value(),
        None => ruby.qnil().as_value(),
    })
}

impl JsonTextValidator {
    fn new_impl(ruby: &Ruby, args: &[Value]) -> Result<Self, Error> {
        let parsed_args = scan_args::<(Value,), (), (), (), _, ()>(args)?;
        let (schema,) = parsed_args.required;
        let kw = extract_kwargs(ruby, parsed_args.keywords)?;
        let json_schema = to_schema_value(ruby, schema)?;
        let parsed = build_parsed_options::<JsonText>(ruby, kw, None)?;
        let BuiltValidator {
            validator,
            callback_roots,
            compilation_roots,
        } = build_validator(
            ruby,
            parsed.options,
            parsed.registry,
            parsed.retriever,
            parsed.callback_roots,
            parsed.compilation_roots,
            &json_schema,
        )?;
        Ok(Self {
            validator,
            mask: parsed.mask,
            callback_roots,
            _compilation_roots: compilation_roots,
        })
    }

    fn is_valid(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<bool, Error> {
        let text = json_text(ruby, instance)?;
        let document = parse(ruby, &text)?;
        catch_unwind_silent(AssertUnwindSafe(|| {
            rb_self.validator.is_valid(document.root())
        }))
        .map_err(|err| handle_callback_panic(ruby, err))
    }

    fn validate(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<(), Error> {
        let text = json_text(ruby, instance)?;
        let document = parse(ruby, &text)?;
        let _scope = KeywordCauseScope::enter();
        match catch_unwind_silent(AssertUnwindSafe(|| {
            rb_self.validator.validate(document.root())
        })) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(raise_validation_error(
                ruby,
                error,
                Some(InstanceRoot::Text(document.root())),
                rb_self.mask.as_deref(),
            )),
            Err(err) => Err(handle_callback_panic(ruby, err)),
        }
    }

    fn iter_errors(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<Value, Error> {
        let text = json_text(ruby, instance)?;
        let document = parse(ruby, &text)?;
        let _scope = KeywordCauseScope::enter();
        let convert = |error| {
            to_ruby_error_value(
                ruby,
                error,
                Some(InstanceRoot::Text(document.root())),
                rb_self.mask.as_deref(),
            )
        };
        if ruby.block_given() {
            // Building the iterator validates, which can run Ruby callbacks.
            let mut errors = catch_unwind_silent(AssertUnwindSafe(|| {
                rb_self.validator.iter_errors(document.root())
            }))
            .map_err(|err| handle_callback_panic(ruby, err))?;
            loop {
                match catch_unwind_silent(AssertUnwindSafe(|| errors.next())) {
                    Ok(Some(error)) => {
                        ruby.yield_value::<Value, Value>(convert(error)?)?;
                    }
                    Ok(None) => return Ok(ruby.qnil().as_value()),
                    Err(err) => return Err(handle_callback_panic(ruby, err)),
                }
            }
        }
        let errors = catch_unwind_silent(AssertUnwindSafe(|| {
            rb_self
                .validator
                .iter_errors(document.root())
                .collect::<Vec<_>>()
        }))
        .map_err(|err| handle_callback_panic(ruby, err))?;
        let array = ruby.ary_new_capa(errors.len());
        for error in errors {
            array.push(convert(error)?)?;
        }
        Ok(array.as_value())
    }

    fn evaluate(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<Evaluation, Error> {
        let text = json_text(ruby, instance)?;
        let document = parse(ruby, &text)?;
        let _scope = KeywordCauseScope::enter();
        catch_unwind_silent(AssertUnwindSafe(|| {
            rb_self.validator.evaluate(document.root())
        }))
        .map(|output| Evaluation::new(output, rb_self.mask.clone()))
        .map_err(|err| handle_callback_panic(ruby, err))
    }

    fn inspect(&self) -> &'static str {
        match self.validator.draft() {
            jsonschema::Draft::Draft4 => "#<JSONSchema::JsonTextValidator: Draft4>",
            jsonschema::Draft::Draft6 => "#<JSONSchema::JsonTextValidator: Draft6>",
            jsonschema::Draft::Draft7 => "#<JSONSchema::JsonTextValidator: Draft7>",
            jsonschema::Draft::Draft201909 => "#<JSONSchema::JsonTextValidator: Draft201909>",
            _ => "#<JSONSchema::JsonTextValidator: Draft202012>",
        }
    }
}

pub fn define_class(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let class = module.define_class("JsonTextValidator", ruby.class_object())?;
    class.define_singleton_method("new", function!(JsonTextValidator::new_impl, -1))?;
    class.define_method("valid?", method!(JsonTextValidator::is_valid, 1))?;
    class.define_method("validate!", method!(JsonTextValidator::validate, 1))?;
    class.define_method("each_error", method!(JsonTextValidator::iter_errors, 1))?;
    class.define_method("evaluate", method!(JsonTextValidator::evaluate, 1))?;
    class.define_method("inspect", method!(JsonTextValidator::inspect, 0))?;
    Ok(())
}
