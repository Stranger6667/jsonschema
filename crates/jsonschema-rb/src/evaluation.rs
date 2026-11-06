//! Evaluation output wrapper for Ruby
//!
//! Provides full JSON Schema output format support: flag, list, and hierarchical.
use jsonschema::{output::ErrorDescription, OutputUnit};
use magnus::{method, prelude::*, Error, RArray, RHash, RModule, Ruby, Value};

use crate::{
    ser::{serialize_to_ruby, value_to_ruby},
    static_id::define_rb_intern,
};

define_rb_intern!(static ID_SCHEMA_LOCATION: "schemaLocation");
define_rb_intern!(static ID_ABSOLUTE_KEYWORD_LOCATION: "absoluteKeywordLocation");
define_rb_intern!(static ID_INSTANCE_LOCATION: "instanceLocation");
define_rb_intern!(static ID_ANNOTATIONS: "annotations");
define_rb_intern!(static ID_ERROR: "error");
define_rb_intern!(static ID_VALID: "valid");
define_rb_intern!(static ID_EVALUATION_PATH: "evaluationPath");
define_rb_intern!(static ID_DROPPED_ANNOTATIONS: "droppedAnnotations");
define_rb_intern!(static ID_ERRORS: "errors");
define_rb_intern!(static ID_DETAILS: "details");

#[magnus::wrap(class = "JSONSchema::Evaluation", free_immediately, size)]
pub struct Evaluation {
    inner: jsonschema::Evaluation,
}

impl Evaluation {
    pub fn new(output: jsonschema::Evaluation) -> Self {
        Evaluation { inner: output }
    }

    fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }

    /// Simplest output format — only a "valid" key.
    fn flag(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let flag_output = rb_self.inner.flag();
        let hash = ruby.hash_new_capa(1);
        hash.aset(ID_VALID.to_symbol(), flag_output.valid)?;
        Ok(hash.as_value())
    }

    /// Flat list of all evaluation results with annotations and errors.
    fn list(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let root = rb_self.inner.root();
        let details = ruby.ary_new();
        list_units(ruby, root, details)?;
        let hash = ruby.hash_new_capa(2);
        hash.aset(ID_VALID.to_symbol(), root.valid())?;
        hash.aset(ID_DETAILS.to_symbol(), details)?;
        Ok(hash.as_value())
    }

    /// Nested tree structure following the schema structure.
    fn hierarchical(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        Ok(unit_to_ruby(ruby, rb_self.inner.root(), true)?.as_value())
    }

    fn annotations(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let schema_loc = ID_SCHEMA_LOCATION.to_symbol();
        let abs_kw_loc = ID_ABSOLUTE_KEYWORD_LOCATION.to_symbol();
        let inst_loc = ID_INSTANCE_LOCATION.to_symbol();
        let annotations_key = ID_ANNOTATIONS.to_symbol();
        let arr = ruby.ary_new();
        for entry in rb_self.inner.iter_annotations() {
            let hash = ruby.hash_new_capa(4);
            hash.aset(schema_loc, entry.schema_location)?;
            if let Some(uri) = entry.absolute_keyword_location {
                hash.aset(abs_kw_loc, uri.as_str())?;
            } else {
                hash.aset(abs_kw_loc, ruby.qnil())?;
            }
            hash.aset(inst_loc, entry.instance_location.as_str())?;
            hash.aset(
                annotations_key,
                value_to_ruby(ruby, entry.annotations.value())?,
            )?;
            arr.push(hash)?;
        }
        Ok(arr.as_value())
    }

    fn errors(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let schema_loc = ID_SCHEMA_LOCATION.to_symbol();
        let abs_kw_loc = ID_ABSOLUTE_KEYWORD_LOCATION.to_symbol();
        let inst_loc = ID_INSTANCE_LOCATION.to_symbol();
        let error_key = ID_ERROR.to_symbol();
        let arr = ruby.ary_new();
        for entry in rb_self.inner.iter_errors() {
            let hash = ruby.hash_new_capa(4);
            hash.aset(schema_loc, entry.schema_location)?;
            if let Some(uri) = entry.absolute_keyword_location {
                hash.aset(abs_kw_loc, uri.as_str())?;
            } else {
                hash.aset(abs_kw_loc, ruby.qnil())?;
            }
            hash.aset(inst_loc, entry.instance_location.as_str())?;
            hash.aset(error_key, entry.error.to_string())?;
            arr.push(hash)?;
        }
        Ok(arr.as_value())
    }

    fn inspect(&self) -> String {
        format!("#<JSONSchema::Evaluation valid={}>", self.inner.is_valid())
    }
}

// Builds an output unit straight into a Ruby Hash, with the keys and order the serialized
// output has.
fn unit_to_ruby(ruby: &Ruby, unit: OutputUnit<'_>, nested: bool) -> Result<RHash, Error> {
    // Key-value pairs inserted with one call, the order the serialized output has.
    let mut pairs = [ruby.qnil().as_value(); 16];
    let mut len = 0;
    let mut push = |key: Value, value: Value| {
        pairs[len] = key;
        pairs[len + 1] = value;
        len += 2;
    };
    push(
        ID_VALID.to_symbol().as_value(),
        ruby.into_value(unit.valid()),
    );
    push(
        ID_EVALUATION_PATH.to_symbol().as_value(),
        ruby.into_value(unit.evaluation_path().as_str()),
    );
    push(
        ID_SCHEMA_LOCATION.to_symbol().as_value(),
        ruby.into_value(unit.schema_location()),
    );
    push(
        ID_INSTANCE_LOCATION.to_symbol().as_value(),
        ruby.into_value(unit.instance_location().as_str()),
    );
    if let Some(annotations) = unit.annotations() {
        push(
            ID_ANNOTATIONS.to_symbol().as_value(),
            serialize_to_ruby(ruby, annotations)?,
        );
    }
    if let Some(annotations) = unit.dropped_annotations() {
        push(
            ID_DROPPED_ANNOTATIONS.to_symbol().as_value(),
            serialize_to_ruby(ruby, annotations)?,
        );
    }
    if !unit.errors().is_empty() {
        push(
            ID_ERRORS.to_symbol().as_value(),
            errors_to_ruby(ruby, unit.errors())?.as_value(),
        );
    }
    if nested {
        let mut children = unit.children().peekable();
        if children.peek().is_some() {
            let details = ruby.ary_new();
            for child in children {
                details.push(unit_to_ruby(ruby, child, true)?)?;
            }
            push(ID_DETAILS.to_symbol().as_value(), details.as_value());
        }
    }
    let hash = ruby.hash_new_capa(len / 2);
    hash.bulk_insert(&pairs[..len])?;
    Ok(hash)
}

fn list_units(ruby: &Ruby, unit: OutputUnit<'_>, out: RArray) -> Result<(), Error> {
    out.push(unit_to_ruby(ruby, unit, false)?)?;
    for child in unit.children() {
        list_units(ruby, child, out)?;
    }
    Ok(())
}

/// Messages grouped by keyword in first-seen order: one message as a String, several as an Array.
fn errors_to_ruby(ruby: &Ruby, errors: &[ErrorDescription]) -> Result<RHash, Error> {
    let hash = ruby.hash_new();
    // ponytail: quadratic in a unit's errors, which hold about one per keyword.
    for (index, error) in errors.iter().enumerate() {
        let keyword = error.keyword();
        if errors[..index].iter().any(|seen| seen.keyword() == keyword) {
            continue;
        }
        let mut messages = errors[index..]
            .iter()
            .filter(|other| other.keyword() == keyword)
            .map(ErrorDescription::message);
        let first = messages.next().expect("the current error has this keyword");
        match messages.next() {
            None => hash.aset(keyword, first)?,
            Some(second) => {
                let all = ruby.ary_from_iter([first, second].into_iter().chain(messages));
                hash.aset(keyword, all)?;
            }
        }
    }
    Ok(hash)
}

pub fn define_class(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let class = module.define_class("Evaluation", ruby.class_object())?;
    class.define_method("valid?", method!(Evaluation::is_valid, 0))?;
    class.define_method("flag", method!(Evaluation::flag, 0))?;
    class.define_method("list", method!(Evaluation::list, 0))?;
    class.define_method("hierarchical", method!(Evaluation::hierarchical, 0))?;
    class.define_method("annotations", method!(Evaluation::annotations, 0))?;
    class.define_method("errors", method!(Evaluation::errors, 0))?;
    class.define_method("inspect", method!(Evaluation::inspect, 0))?;

    Ok(())
}
