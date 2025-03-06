mod any;
mod array;
mod combinators;
mod numeric;
mod object;
mod string;

use any::TypeSet;
use array::{MaxItems, MinItems};
use numeric::{ExclusiveMaximum, ExclusiveMinimum, Maximum, Minimum};
use object::{MaxProperties, MinProperties, Required};
use serde_json::{Number, Value};
use string::{MaxLength, MinLength, MinMaxLength};

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub(crate) enum EvaluationScope {
    // And (allOf) states
    AndValid = 0,   // All conditions true so far
    AndInvalid = 1, // At least one condition false

    // Or (anyOf) states
    OrSearching = 2, // Still looking for a valid branch
    OrSatisfied = 3, // At least one valid branch found

    // Xor (oneOf) states
    XorEmpty = 4,    // No valid branch found yet
    XorSingle = 5,   // Exactly one valid branch found
    XorMultiple = 6, // More than one valid branch found
}

#[derive(Debug)]
pub(crate) enum Instruction {
    True,
    False,

    TypeSet(TypeSet),
    TypeNull,
    TypeBoolean,
    TypeString,
    TypeNumber,
    TypeInteger,
    TypeArray,
    TypeObject,

    MaximumU64(Maximum<u64>),
    MaximumI64(Maximum<i64>),
    MaximumF64(Maximum<f64>),
    MinimumU64(Minimum<u64>),
    MinimumI64(Minimum<i64>),
    MinimumF64(Minimum<f64>),
    ExclusiveMaximumU64(ExclusiveMaximum<u64>),
    ExclusiveMaximumI64(ExclusiveMaximum<i64>),
    ExclusiveMaximumF64(ExclusiveMaximum<f64>),
    ExclusiveMinimumU64(ExclusiveMinimum<u64>),
    ExclusiveMinimumI64(ExclusiveMinimum<i64>),
    ExclusiveMinimumF64(ExclusiveMinimum<f64>),

    MaxLength(MaxLength),
    MinLength(MinLength),
    MinMaxLength(MinMaxLength),

    MinProperties(MinProperties),
    MaxProperties(MaxProperties),
    Required(Required),

    MinItems(MinItems),
    MaxItems(MaxItems),

    JumpBackward(usize),

    ArrayIter(usize),
    ArrayIterNext(usize),

    ObjectValuesIter(usize),
    ObjectValuesIterNext(usize),

    PushProperty {
        name: Box<str>,
        skip_if_missing: usize,
    },
    PopValue,

    PushScope(EvaluationScope),
    PopScope,
}

macro_rules! impl_conversions {
    ($($from_type:ty => $instruction_variant:ident),*) => {
        $(
            impl From<$from_type> for Instruction {
                fn from(val: $from_type) -> Self {
                    Instruction::$instruction_variant(val)
                }
            }
        )*
    };
}

impl_conversions!(
    TypeSet => TypeSet,

    Minimum<u64> => MinimumU64,
    Minimum<i64> => MinimumI64,
    Minimum<f64> => MinimumF64,
    Maximum<u64> => MaximumU64,
    Maximum<i64> => MaximumI64,
    Maximum<f64> => MaximumF64,
    ExclusiveMinimum<u64> => ExclusiveMinimumU64,
    ExclusiveMinimum<i64> => ExclusiveMinimumI64,
    ExclusiveMinimum<f64> => ExclusiveMinimumF64,
    ExclusiveMaximum<u64> => ExclusiveMaximumU64,
    ExclusiveMaximum<i64> => ExclusiveMaximumI64,
    ExclusiveMaximum<f64> => ExclusiveMaximumF64,

    MinLength => MinLength,
    MaxLength => MaxLength,
    MinMaxLength => MinMaxLength,

    MinProperties => MinProperties,
    MaxProperties => MaxProperties,
    Required => Required,

    MinItems => MinItems,
    MaxItems => MaxItems
);

pub(crate) struct SchemaCompiler {
    instructions: Vec<Instruction>,
}

macro_rules! emit_jump {
    ($compiler:expr, $variant:ident) => {{
        let idx = $compiler.instructions.len();
        $compiler.instructions.push(Instruction::$variant(0));
        idx
    }};
}

macro_rules! patch_jump {
    ($compiler:expr, $idx:expr, $variant:ident) => {{
        let current_idx = $compiler.instructions.len();
        if let Instruction::$variant(ref mut offset) = $compiler.instructions[$idx] {
            *offset = current_idx - $idx - 1;
        } else {
            panic!(
                "Expected {} instruction at position {}",
                stringify!($variant),
                $idx
            );
        }
    }};
}

macro_rules! define_jumps {
    ($($emit_name:ident, $patch_name:ident => $variant:ident),* $(,)?) => {
        $(
            pub(crate) fn $emit_name(&mut self) -> usize {
                emit_jump!(self, $variant)
            }
            pub(crate) fn $patch_name(&mut self, idx: usize) {
                patch_jump!(self, idx, $variant)
            }
        )*
    }
}

impl SchemaCompiler {
    pub(crate) fn new() -> Self {
        Self {
            instructions: Vec::new(),
        }
    }

    fn emit(&mut self, instruction: impl Into<Instruction>) {
        self.instructions.push(instruction.into());
    }

    pub(crate) fn emit_jump_backward(&mut self, target_idx: usize) {
        let current_idx = self.instructions.len();
        self.emit(Instruction::JumpBackward(current_idx - target_idx));
    }

    define_jumps!(
        emit_array_iter, patch_array_iter => ArrayIter,
        emit_array_iter_next, patch_array_iter_next => ArrayIterNext,
        emit_object_values_iter, patch_object_values_iter => ObjectValuesIter,
        emit_object_values_iter_next, patch_object_values_iter_next => ObjectValuesIterNext,
    );

    pub(crate) fn compile(schema: &Value) -> Vec<Instruction> {
        let mut compiler = Self::new();
        compiler.compile_impl(schema);
        compiler.instructions
    }

    pub(crate) fn compile_impl(&mut self, schema: &Value) {
        match schema {
            Value::Bool(true) => {
                self.emit(Instruction::True);
            }
            Value::Bool(false) => {
                self.emit(Instruction::False);
            }
            Value::Object(obj) => {
                if obj.is_empty() {
                    self.emit(Instruction::True);
                } else {
                    combinators::compile(self, obj);
                    any::compile(self, obj);
                    string::compile(self, obj);
                    numeric::compile(self, obj);
                    array::compile(self, obj);
                    object::compile(self, obj);
                }
            }
            _ => panic!("Invalid schema: expected object or boolean"),
        }
    }

    fn compile_integer<F, I>(&mut self, value: &Number, constructor: F) -> bool
    where
        F: FnOnce(usize) -> I,
        I: Into<Instruction>,
    {
        if let Some(limit) = value.as_u64() {
            self.emit(constructor(limit as usize));
            true
        } else if let Some(limit) = value.as_f64() {
            if limit.trunc() == limit {
                self.emit(constructor(limit as usize));
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}
