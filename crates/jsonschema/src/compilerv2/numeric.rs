use num_cmp::NumCmp;
use serde_json::{Map, Number, Value};

use super::SchemaCompiler;

#[derive(Debug, Clone)]
pub(crate) struct Minimum<T> {
    limit: T,
}

impl<T> Minimum<T>
where
    T: Copy,
    u64: NumCmp<T>,
    i64: NumCmp<T>,
    f64: NumCmp<T>,
{
    pub(crate) fn new(limit: T) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, value: &Number) -> bool {
        if let Some(v) = value.as_u64() {
            return !NumCmp::num_lt(v, self.limit);
        }
        if let Some(v) = value.as_i64() {
            return !NumCmp::num_lt(v, self.limit);
        }
        let v = value.as_f64().expect("Always valid");
        !NumCmp::num_lt(v, self.limit)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExclusiveMinimum<T> {
    limit: T,
}

impl<T> ExclusiveMinimum<T>
where
    T: Copy,
    u64: NumCmp<T>,
    i64: NumCmp<T>,
    f64: NumCmp<T>,
{
    pub(crate) fn new(limit: T) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, value: &Number) -> bool {
        if let Some(v) = value.as_u64() {
            return NumCmp::num_gt(v, self.limit);
        }
        if let Some(v) = value.as_i64() {
            return NumCmp::num_gt(v, self.limit);
        }
        let v = value.as_f64().expect("Always valid");
        NumCmp::num_gt(v, self.limit)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Maximum<T> {
    limit: T,
}

impl<T> Maximum<T>
where
    T: Copy,
    u64: NumCmp<T>,
    i64: NumCmp<T>,
    f64: NumCmp<T>,
{
    pub(crate) fn new(limit: T) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, value: &Number) -> bool {
        if let Some(v) = value.as_u64() {
            return !NumCmp::num_gt(v, self.limit);
        }
        if let Some(v) = value.as_i64() {
            return !NumCmp::num_gt(v, self.limit);
        }
        let v = value.as_f64().expect("Always valid");
        !NumCmp::num_gt(v, self.limit)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExclusiveMaximum<T> {
    limit: T,
}

impl<T> ExclusiveMaximum<T>
where
    T: Copy,
    u64: NumCmp<T>,
    i64: NumCmp<T>,
    f64: NumCmp<T>,
{
    pub(crate) fn new(limit: T) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, value: &Number) -> bool {
        if let Some(v) = value.as_u64() {
            return NumCmp::num_lt(v, self.limit);
        }
        if let Some(v) = value.as_i64() {
            return NumCmp::num_lt(v, self.limit);
        }
        let v = value.as_f64().expect("Always valid");
        NumCmp::num_lt(v, self.limit) // Must be < (not <=)
    }
}

macro_rules! compile_numeric {
    ($compiler:expr, $value:expr, $constructor:expr) => {
        if let Some(limit) = $value.as_u64() {
            $compiler.emit($constructor(limit));
        } else if let Some(limit) = $value.as_i64() {
            $compiler.emit($constructor(limit));
        } else if let Some(limit) = $value.as_f64() {
            $compiler.emit($constructor(limit));
        }
    };
}

pub(super) fn compile(compiler: &mut SchemaCompiler, obj: &Map<String, Value>) {
    if let Some(Value::Number(value)) = obj.get("minimum") {
        compile_numeric!(compiler, value, Minimum::new);
    }
    if let Some(Value::Number(value)) = obj.get("exclusiveMinimum") {
        compile_numeric!(compiler, value, ExclusiveMinimum::new);
    }
    if let Some(Value::Number(value)) = obj.get("maximum") {
        compile_numeric!(compiler, value, Maximum::new);
    }
    if let Some(Value::Number(value)) = obj.get("exclusiveMaximum") {
        compile_numeric!(compiler, value, ExclusiveMaximum::new);
    }

    // TODO: multipleOf, etc.
}
