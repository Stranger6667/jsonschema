use serde_json::{Map, Value};

use super::SchemaCompiler;

#[derive(Debug, Clone)]
pub(crate) struct MinLength {
    pub(super) limit: usize,
}

impl MinLength {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, value: &str) -> bool {
        value.chars().count() >= self.limit
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MaxLength {
    pub(super) limit: usize,
}

impl MaxLength {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, value: &str) -> bool {
        value.chars().count() <= self.limit
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MinMaxLength {
    minimum: usize,
    maximum: usize,
}

impl MinMaxLength {
    pub(crate) fn new(minimum: usize, maximum: usize) -> Self {
        Self { minimum, maximum }
    }
    pub(crate) fn execute(&self, value: &str) -> bool {
        let length = value.chars().count();
        length >= self.minimum && length <= self.maximum
    }
}

pub(super) fn compile(
    compiler: &mut SchemaCompiler,
    obj: &Map<String, Value>,
    jumps: &mut Vec<usize>,
) {
    let min_length = obj.get("minLength").and_then(Value::as_number);
    let max_length = obj.get("maxLength").and_then(Value::as_number);

    match (min_length, max_length) {
        (Some(min), Some(max)) => match (min.as_u64(), max.as_u64()) {
            (Some(min), Some(max)) => {
                compiler.emit(MinMaxLength::new(min as usize, max as usize));
                jumps.push(compiler.emit_jump_if_invalid());
            }
            _ => todo!(),
        },
        (Some(min), None) => {
            if !compiler.compile_integer(min, MinLength::new) {
                todo!("Non-integer minLength");
            }
            jumps.push(compiler.emit_jump_if_invalid());
        }
        (None, Some(max)) => {
            if !compiler.compile_integer(max, MaxLength::new) {
                todo!("Non-integer maxLength");
            }
            jumps.push(compiler.emit_jump_if_invalid());
        }
        _ => {}
    }

    // TODO: Add other string keywords (pattern, format, etc.)
}
