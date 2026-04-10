//! ValidatorMap binding for Ruby.
use magnus::{gc::Marker, method, prelude::*, DataTypeFunctions, Error, RModule, Ruby, TypedData};

use crate::{
    options::{CallbackRoots, CompilationRootsRef},
    Validator,
};

/// Ruby wrapper around `jsonschema::ValidatorMap`.
#[derive(TypedData)]
#[magnus(class = "JSONSchema::ValidatorMap", free_immediately, size, mark)]
pub struct ValidatorMap {
    pub(crate) inner: jsonschema::ValidatorMap,
    pub(crate) has_ruby_callbacks: bool,
    pub(crate) callback_roots: CallbackRoots,
    pub(crate) compilation_roots: CompilationRootsRef,
    pub(crate) mask: Option<String>,
}

impl DataTypeFunctions for ValidatorMap {
    fn mark(&self, marker: &Marker) {
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

impl ValidatorMap {
    /// `map["#/$defs/User"]` → Validator | nil
    #[allow(clippy::needless_pass_by_value)]
    fn get(&self, pointer: String) -> Option<Validator> {
        self.inner.get(&pointer).map(|v| {
            Validator::from_jsonschema_with_roots(
                v.clone(),
                self.mask.clone(),
                self.has_ruby_callbacks,
                self.callback_roots.clone(),
                self.compilation_roots.clone(),
            )
        })
    }

    /// `map.fetch("#/$defs/User")` → Validator (raises KeyError if missing)
    #[allow(clippy::needless_pass_by_value)]
    fn fetch(ruby: &Ruby, rb_self: &Self, pointer: String) -> Result<Validator, Error> {
        match rb_self.inner.get(&pointer) {
            Some(v) => Ok(Validator::from_jsonschema_with_roots(
                v.clone(),
                rb_self.mask.clone(),
                rb_self.has_ruby_callbacks,
                rb_self.callback_roots.clone(),
                rb_self.compilation_roots.clone(),
            )),
            None => Err(Error::new(
                ruby.exception_key_error(),
                format!("key not found: {pointer}"),
            )),
        }
    }

    /// `map.key?("#/$defs/User")` → bool
    #[allow(clippy::needless_pass_by_value)]
    fn key_p(&self, pointer: String) -> bool {
        self.inner.contains_key(&pointer)
    }

    /// `map.keys` → Array[String]
    fn keys(&self) -> Vec<String> {
        self.inner.keys().map(str::to_owned).collect()
    }

    /// `map.length` → Integer
    fn length(&self) -> usize {
        self.inner.len()
    }
}

pub fn define_class(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let class = module.define_class("ValidatorMap", ruby.class_object())?;
    class.define_method("[]", method!(ValidatorMap::get, 1))?;
    class.define_method("fetch", method!(ValidatorMap::fetch, 1))?;
    class.define_method("key?", method!(ValidatorMap::key_p, 1))?;
    class.define_method("keys", method!(ValidatorMap::keys, 0))?;
    class.define_method("length", method!(ValidatorMap::length, 0))?;
    class.define_alias("size", "length")?;
    Ok(())
}
