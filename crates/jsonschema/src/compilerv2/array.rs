use serde_json::{Map, Value};

use super::{Instruction, SchemaCompiler};

#[derive(Debug, Clone)]
pub(crate) struct MinItems {
    limit: usize,
}

impl MinItems {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, arr: &[Value]) -> bool {
        arr.len() >= self.limit
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MaxItems {
    limit: usize,
}

impl MaxItems {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, arr: &[Value]) -> bool {
        arr.len() <= self.limit
    }
}

pub(super) fn compile(compiler: &mut SchemaCompiler, obj: &Map<String, Value>) {
    if let Some(Value::Number(value)) = obj.get("minItems") {
        if !compiler.compile_integer(value, MinItems::new) {
            todo!("Non-integer minItems");
        }
    }
    if let Some(Value::Number(value)) = obj.get("maxItems") {
        if !compiler.compile_integer(value, MaxItems::new) {
            todo!("Non-integer maxItems");
        }
    }
    if let Some(items) = obj.get("items") {
        if items.is_object() {
            let iter_idx = compiler.emit_array_iter();
            let next_idx = compiler.emit_array_iter_next();
            compiler.compile_impl(items);
            compiler.emit_jump_backward(iter_idx);
            compiler.patch_array_iter_next(next_idx);
            compiler.patch_array_iter(iter_idx);
        }
    }
}
