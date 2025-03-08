use fraction::{BigFraction, BigUint};
use num_cmp::NumCmp;
use serde_json::{Map, Number, Value};

use super::SchemaCompiler;

#[derive(Debug, Clone)]
pub(crate) struct Minimum<T> {
    pub(super) limit: T,
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
    pub(super) limit: T,
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
    pub(super) limit: T,
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
    pub(super) limit: T,
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

#[derive(Debug, Clone)]
pub struct MultipleOfInteger {
    pub(super) multiple_of: f64,
}

impl MultipleOfInteger {
    pub fn new(multiple_of: f64) -> MultipleOfInteger {
        Self { multiple_of }
    }

    pub fn execute(&self, value: &Number) -> bool {
        let item = value.as_f64().expect("Always valid");
        // As the divisor has its fractional part as zero, then any value with a non-zero
        // fractional part can't be a multiple of this divisor, therefore it is short-circuited
        item.fract() == 0. && (item % self.multiple_of) == 0.
    }
}

#[derive(Debug, Clone)]
pub struct MultipleOfFloat {
    pub(super) multiple_of: f64,
}

impl MultipleOfFloat {
    pub fn new(multiple_of: f64) -> MultipleOfFloat {
        Self { multiple_of }
    }

    pub fn execute(&self, value: &Number) -> bool {
        let item = value.as_f64().expect("Always valid");
        let remainder = (item / self.multiple_of) % 1.;
        if remainder.is_nan() {
            // Involves heap allocations via the underlying `BigUint` type
            let fraction = BigFraction::from(item) / BigFraction::from(self.multiple_of);
            if let Some(denom) = fraction.denom() {
                denom == &BigUint::from(1_u8)
            } else {
                true
            }
        } else {
            remainder < f64::EPSILON
        }
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

pub(super) fn compile(
    compiler: &mut SchemaCompiler,
    obj: &Map<String, Value>,
    jumps: &mut Vec<usize>,
) {
    if let Some(Value::Number(value)) = obj.get("minimum") {
        compile_numeric!(compiler, value, Minimum::new);
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Number(value)) = obj.get("exclusiveMinimum") {
        compile_numeric!(compiler, value, ExclusiveMinimum::new);
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Number(value)) = obj.get("maximum") {
        compile_numeric!(compiler, value, Maximum::new);
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Number(value)) = obj.get("exclusiveMaximum") {
        compile_numeric!(compiler, value, ExclusiveMaximum::new);
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Number(value)) = obj.get("multipleOf") {
        let multiple_of = value.as_f64().expect("Always valid");
        if multiple_of.fract() == 0. {
            compiler.emit(MultipleOfInteger::new(multiple_of))
        } else {
            compiler.emit(MultipleOfFloat::new(multiple_of))
        }
        jumps.push(compiler.emit_jump_if_invalid());
    }
}
