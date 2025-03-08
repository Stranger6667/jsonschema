mod any;
mod array;
mod combinators;
mod numeric;
mod object;
mod string;

use core::fmt;

use any::{Enum, EnumSingle, TypeSet};
use array::{MaxItems, MinItems};
use numeric::{
    ExclusiveMaximum, ExclusiveMinimum, Maximum, Minimum, MultipleOfFloat, MultipleOfInteger,
};
use object::{MaxProperties, MinProperties, Required};
use serde_json::{Number, Value};
use string::{MaxLength, MinLength, MinMaxLength};

pub(crate) use array::is_unique;
pub(crate) use combinators::OneOfStack;

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
    Enum(Enum),
    EnumSingle(EnumSingle),

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
    MultipleOfInteger(MultipleOfInteger),
    MultipleOfFloat(MultipleOfFloat),

    MaxLength(MaxLength),
    MinLength(MinLength),
    MinMaxLength(MinMaxLength),

    MinProperties(MinProperties),
    MaxProperties(MaxProperties),
    Required(Required),

    MinItems(MinItems),
    MaxItems(MaxItems),
    UniqueItems,

    ArrayIter(usize),
    ArrayIterNext(usize),

    ObjectValuesIter(usize),
    ObjectValuesIterNext(usize),

    PushProperty {
        name: Box<str>,
        skip_if_missing: usize,
        required: bool,
    },
    PopValue,

    JumpIfValid(usize),
    JumpIfSecondValid(usize),
    JumpIfInvalid(usize),

    PushOneOf,
    SetOneValid,
    PopOneOf,
}

impl fmt::Debug for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Instruction::True => write!(f, "TRUE"),
            Instruction::False => write!(f, "FALSE"),

            Instruction::TypeSet(ts) => write!(f, "TYPE_SET {:?}", ts),
            Instruction::TypeNull => write!(f, "TYPE_NULL"),
            Instruction::TypeBoolean => write!(f, "TYPE_BOOLEAN"),
            Instruction::TypeString => write!(f, "TYPE_STRING"),
            Instruction::TypeNumber => write!(f, "TYPE_NUMBER"),
            Instruction::TypeInteger => write!(f, "TYPE_INTEGER"),
            Instruction::TypeArray => write!(f, "TYPE_ARRAY"),
            Instruction::TypeObject => write!(f, "TYPE_OBJECT"),
            Instruction::Enum(e) => write!(f, "ENUM {:?}", e),
            Instruction::EnumSingle(es) => write!(f, "ENUM_SINGLE {:?}", es),

            Instruction::MaximumU64(m) => write!(f, "MAXIMUM_U64 {:?}", m.limit),
            Instruction::MaximumI64(m) => write!(f, "MAXIMUM_I64 {:?}", m.limit),
            Instruction::MaximumF64(m) => write!(f, "MAXIMUM_F64 {:?}", m.limit),
            Instruction::MinimumU64(m) => write!(f, "MINIMUM_U64 {:?}", m.limit),
            Instruction::MinimumI64(m) => write!(f, "MINIMUM_I64 {:?}", m.limit),
            Instruction::MinimumF64(m) => write!(f, "MINIMUM_F64 {:?}", m.limit),
            Instruction::ExclusiveMaximumU64(m) => write!(f, "EXCLUSIVE_MAXIMUM_U64 {:?}", m.limit),
            Instruction::ExclusiveMaximumI64(m) => write!(f, "EXCLUSIVE_MAXIMUM_I64 {:?}", m.limit),
            Instruction::ExclusiveMaximumF64(m) => write!(f, "EXCLUSIVE_MAXIMUM_F64 {:?}", m.limit),
            Instruction::ExclusiveMinimumU64(m) => write!(f, "EXCLUSIVE_MINIMUM_U64 {:?}", m.limit),
            Instruction::ExclusiveMinimumI64(m) => write!(f, "EXCLUSIVE_MINIMUM_I64 {:?}", m.limit),
            Instruction::ExclusiveMinimumF64(m) => write!(f, "EXCLUSIVE_MINIMUM_F64 {:?}", m.limit),
            Instruction::MultipleOfInteger(m) => {
                write!(f, "MULTIPLE_OF_INTEGER {:?}", m.multiple_of)
            }
            Instruction::MultipleOfFloat(m) => write!(f, "MULTIPLE_OF_FLOAT {:?}", m.multiple_of),

            Instruction::MaxLength(m) => write!(f, "MAX_LENGTH {:?}", m.limit),
            Instruction::MinLength(m) => write!(f, "MIN_LENGTH {:?}", m.limit),
            Instruction::MinMaxLength(m) => write!(f, "MIN_MAX_LENGTH {:?}", m),

            Instruction::MinProperties(m) => write!(f, "MIN_PROPERTIES {:?}", m.limit),
            Instruction::MaxProperties(m) => write!(f, "MAX_PROPERTIES {:?}", m.limit),
            Instruction::Required(r) => write!(f, "REQUIRED {:?}", r.required),

            Instruction::MinItems(m) => write!(f, "MIN_ITEMS {:?}", m),
            Instruction::MaxItems(m) => write!(f, "MAX_ITEMS {:?}", m),
            Instruction::UniqueItems => write!(f, "UNIQUE_ITEMS"),

            Instruction::ArrayIter(idx) => write!(f, "ARRAY_ITER {}", idx),
            Instruction::ArrayIterNext(idx) => write!(f, "ARRAY_ITER_NEXT {}", idx),

            Instruction::ObjectValuesIter(idx) => write!(f, "OBJECT_VALUES_ITER {}", idx),
            Instruction::ObjectValuesIterNext(idx) => write!(f, "OBJECT_VALUES_ITER_NEXT {}", idx),

            Instruction::PushProperty {
                name,
                skip_if_missing,
                required,
            } => {
                write!(
                    f,
                    "PUSH_PROPERTY name={} skip_if_missing={} required={}",
                    name, skip_if_missing, required
                )
            }
            Instruction::PopValue => write!(f, "POP_VALUE"),

            Instruction::JumpIfValid(addr) => write!(f, "JUMP_IF_VALID {}", addr),
            Instruction::JumpIfSecondValid(addr) => write!(f, "JUMP_IF_SECOND_VALID {}", addr),
            Instruction::JumpIfInvalid(addr) => write!(f, "JUMP_IF_INVALID {}", addr),

            Instruction::PushOneOf => write!(f, "PUSH_ONE_OF"),
            Instruction::SetOneValid => write!(f, "SET_ONE_VALID"),
            Instruction::PopOneOf => write!(f, "POP_ONE_OF"),
        }
    }
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
    Enum => Enum,
    EnumSingle => EnumSingle,

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
    MultipleOfInteger => MultipleOfInteger,
    MultipleOfFloat => MultipleOfFloat,

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
        let idx = $compiler.current_location();
        $compiler.instructions.push(Instruction::$variant(0));
        idx
    }};
}

macro_rules! patch_jump {
    ($compiler:expr, $idx:expr, $variant:ident) => {{
        let current_idx = $compiler.current_location();
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

    fn current_location(&self) -> usize {
        self.instructions.len()
    }

    fn emit(&mut self, instruction: impl Into<Instruction>) {
        self.instructions.push(instruction.into());
    }

    pub(crate) fn emit_array_iter_next(&mut self, target_idx: usize) {
        self.emit(Instruction::ArrayIterNext(
            self.current_location() - target_idx - 1,
        ));
    }

    pub(crate) fn emit_object_values_iter_next(&mut self, target_idx: usize) {
        self.emit(Instruction::ObjectValuesIterNext(
            self.current_location() - target_idx - 1,
        ));
    }

    pub(crate) fn emit_true(&mut self) {
        self.emit(Instruction::True)
    }

    pub(crate) fn emit_false(&mut self) {
        self.emit(Instruction::False)
    }

    pub(crate) fn emit_push_one_of(&mut self) {
        self.emit(Instruction::PushOneOf)
    }

    pub(crate) fn emit_pop_one_of(&mut self) {
        self.emit(Instruction::PopOneOf)
    }

    pub(crate) fn emit_set_one_valid(&mut self) {
        self.emit(Instruction::SetOneValid)
    }

    pub(crate) fn emit_unique_items(&mut self) {
        self.emit(Instruction::UniqueItems)
    }

    define_jumps!(
        emit_jump_if_valid, patch_jump_if_valid => JumpIfValid,
        emit_jump_if_second_valid, patch_jump_if_second_valid => JumpIfSecondValid,
        emit_jump_if_invalid, patch_jump_if_invalid => JumpIfInvalid,
        emit_array_iter, patch_array_iter => ArrayIter,
        emit_object_values_iter, patch_object_values_iter => ObjectValuesIter,
    );

    pub(crate) fn compile(schema: &Value) -> Vec<Instruction> {
        let mut compiler = Self::new();
        compiler.compile_schema(schema);
        compiler.instructions
    }

    pub(crate) fn compile_schema(&mut self, schema: &Value) {
        match schema {
            Value::Bool(true) => self.emit_true(),
            Value::Bool(false) => self.emit_false(),
            Value::Object(obj) => {
                if obj.is_empty() {
                    self.emit_true();
                } else {
                    let mut jumps = vec![];
                    combinators::compile(self, obj, &mut jumps);
                    any::compile(self, obj, &mut jumps);
                    string::compile(self, obj, &mut jumps);
                    numeric::compile(self, obj, &mut jumps);
                    array::compile(self, obj, &mut jumps);
                    object::compile(self, obj, &mut jumps);
                    match jumps.as_slice() {
                        [_] if matches!(
                            self.instructions.last(),
                            Some(Instruction::JumpIfInvalid(_))
                        ) =>
                        {
                            self.instructions.pop();
                        }
                        [jumps @ .., _]
                            if matches!(
                                self.instructions.last(),
                                Some(Instruction::JumpIfInvalid(_))
                            ) =>
                        {
                            self.instructions.pop();
                            for jump in jumps {
                                self.patch_jump_if_invalid(*jump);
                            }
                        }
                        jumps => {
                            for jump in jumps {
                                self.patch_jump_if_invalid(*jump);
                            }
                        }
                    }
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
