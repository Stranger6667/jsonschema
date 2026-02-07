//! Evaluation output wrapper for Ruby
//!
//! Provides full JSON Schema output format support: flag, list, and hierarchical.
use magnus::{method, prelude::*, Error, RModule, Ruby, Value};

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

#[magnus::wrap(class = "JSONSchema::Evaluation", free_immediately, size)]
pub struct Evaluation {
    inner: jsonschema::Evaluation,
}

impl Evaluation {
    pub fn new(output: jsonschema::Evaluation) -> Self {
        Evaluation { inner: output }
    }

    fn is_valid(&self) -> bool {
        self.inner.flag().valid
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
        let list_output = rb_self.inner.list();
        serialize_to_ruby(ruby, &list_output)
    }

    /// Nested tree structure following the schema structure.
    fn hierarchical(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let hierarchical_output = rb_self.inner.hierarchical();
        serialize_to_ruby(ruby, &hierarchical_output)
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
        format!(
            "#<JSONSchema::Evaluation valid={}>",
            self.inner.flag().valid
        )
    }
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
