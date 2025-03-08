use ahash::HashSet;
use serde_json::{Map, Value};

use super::{Instruction, SchemaCompiler};

#[derive(Debug, Clone)]
pub(crate) struct MinProperties {
    pub(super) limit: usize,
}

impl MinProperties {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, obj: &Map<String, Value>) -> bool {
        obj.len() >= self.limit
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MaxProperties {
    pub(super) limit: usize,
}

impl MaxProperties {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, obj: &Map<String, Value>) -> bool {
        obj.len() <= self.limit
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Required {
    pub(super) required: Vec<String>,
}

impl Required {
    pub(crate) fn new(required: Vec<String>) -> Self {
        Self { required }
    }

    pub(crate) fn execute(&self, obj: &Map<String, Value>) -> bool {
        self.required.iter().all(|key| obj.contains_key(key))
    }
}

pub(super) fn compile(
    compiler: &mut SchemaCompiler,
    obj: &Map<String, Value>,
    jumps: &mut Vec<usize>,
) {
    if let Some(Value::Number(value)) = obj.get("minProperties") {
        if !compiler.compile_integer(value, MinProperties::new) {
            todo!("Non-integer minProperties");
        }
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Number(value)) = obj.get("maxProperties") {
        if !compiler.compile_integer(value, MaxProperties::new) {
            todo!("Non-integer maxProperties");
        }
        jumps.push(compiler.emit_jump_if_invalid());
    }

    let mut required_properties = HashSet::default();

    if let Some(Value::Array(properties)) = obj.get("required") {
        for prop in properties {
            if let Value::String(name) = prop {
                required_properties.insert(name.clone());
            } else {
                panic!("Required property name must be a string");
            }
        }
    }

    if let Some(Value::Object(value)) = obj.get("properties") {
        for (key, schema) in value {
            let jump_idx = compiler.instructions.len();

            // Check if this property is in the required set
            let is_required = required_properties.contains(key);

            // If it's required, remove it from the set of properties we need to check later
            if is_required {
                required_properties.remove(key);
            }

            compiler.emit(Instruction::PushProperty {
                name: key.clone().into_boxed_str(),
                skip_if_missing: 0,
                required: is_required,
            });

            compiler.compile_schema(schema);
            compiler.emit(Instruction::PopValue);

            let current_idx = compiler.instructions.len();
            if let Instruction::PushProperty {
                skip_if_missing, ..
            } = &mut compiler.instructions[jump_idx]
            {
                *skip_if_missing = current_idx - jump_idx - 1;
            }
            jumps.push(compiler.emit_jump_if_invalid());
        }
    }
    if !required_properties.is_empty() {
        let remaining_required: Vec<String> = required_properties.into_iter().collect();
        compiler.emit(Required::new(remaining_required));
        jumps.push(compiler.emit_jump_if_invalid());
    }

    if let Some(properties) = obj.get("additionalProperties") {
        if properties.is_object() {
            let iter_idx = compiler.emit_object_values_iter();
            compiler.compile_schema(properties);
            compiler.emit_object_values_iter_next(iter_idx);
            compiler.patch_object_values_iter(iter_idx);
            jumps.push(compiler.emit_jump_if_invalid());
        }
    }

    // TODO: additionalProperties, patternProperties, dependencies, etc.
}
