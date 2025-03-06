use serde_json::{Map, Value};

use super::{Instruction, SchemaCompiler};

#[derive(Debug, Clone)]
pub(crate) struct MinProperties {
    limit: usize,
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
    limit: usize,
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
    required: Vec<String>,
}

impl Required {
    pub(crate) fn new(required: Vec<String>) -> Self {
        Self { required }
    }

    pub(crate) fn execute(&self, obj: &Map<String, Value>) -> bool {
        self.required.iter().all(|key| obj.contains_key(key))
    }
}

pub(super) fn compile(compiler: &mut SchemaCompiler, obj: &Map<String, Value>) {
    if let Some(Value::Number(value)) = obj.get("minProperties") {
        if !compiler.compile_integer(value, MinProperties::new) {
            todo!("Non-integer minProperties");
        }
    }
    if let Some(Value::Number(value)) = obj.get("maxProperties") {
        if !compiler.compile_integer(value, MaxProperties::new) {
            todo!("Non-integer maxProperties");
        }
    }
    if let Some(Value::Object(value)) = obj.get("properties") {
        for (key, schema) in value {
            let jump_idx = compiler.instructions.len();
            compiler.emit(Instruction::PushProperty {
                name: key.clone().into_boxed_str(),
                skip_if_missing: 0,
            });

            compiler.compile_impl(schema);
            compiler.emit(Instruction::PopValue);

            let current_idx = compiler.instructions.len();
            if let Instruction::PushProperty {
                skip_if_missing, ..
            } = &mut compiler.instructions[jump_idx]
            {
                *skip_if_missing = current_idx - jump_idx - 1;
            }
        }
    }
    if let Some(Value::Array(properties)) = obj.get("required") {
        let mut property_names = Vec::with_capacity(properties.len());

        for prop in properties {
            if let Value::String(name) = prop {
                property_names.push(name.clone());
            } else {
                // The JSON Schema spec requires required property names to be strings
                panic!("Required property name must be a string");
            }
        }

        if !property_names.is_empty() {
            compiler.emit(Required::new(property_names));
        }
    }

    if let Some(properties) = obj.get("additionalProperties") {
        if properties.is_object() {
            let iter_idx = compiler.emit_object_values_iter();
            let next_idx = compiler.emit_object_values_iter_next();
            compiler.compile_impl(properties);
            compiler.emit_jump_backward(iter_idx);
            compiler.patch_object_values_iter_next(next_idx);
            compiler.patch_object_values_iter(iter_idx);
        }
    }

    // TODO: additionalProperties, patternProperties, dependencies, etc.
}
