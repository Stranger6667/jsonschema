use crate::paths::Location;

use super::{numeric, types::JsonTypeSet};

pub(super) type InstructionIdx = u32;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Instruction {
    TypeNull,
    TypeBoolean,
    TypeNumber,
    TypeInteger,
    TypeString,
    TypeObject,
    TypeArray,
    TypeSet(JsonTypeSet),

    MinimumU64(numeric::Minimum<u64>),
    MinimumI64(numeric::Minimum<i64>),
    MinimumF64(numeric::Minimum<f64>),
    MaximumU64(numeric::Maximum<u64>),
    MaximumI64(numeric::Maximum<i64>),
    MaximumF64(numeric::Maximum<f64>),
    ExclusiveMinimumU64(numeric::ExclusiveMinimum<u64>),
    ExclusiveMinimumI64(numeric::ExclusiveMinimum<i64>),
    ExclusiveMinimumF64(numeric::ExclusiveMinimum<f64>),
    ExclusiveMaximumU64(numeric::ExclusiveMaximum<u64>),
    ExclusiveMaximumI64(numeric::ExclusiveMaximum<i64>),
    ExclusiveMaximumF64(numeric::ExclusiveMaximum<f64>),
    MultipleOfFloat(numeric::MultipleOfFloat),
    MultipleOfInteger(numeric::MultipleOfInteger),

    True,
    False,

    JumpIfFalseOrPop(u32),
    JumpIfTrueOrPop(u32),
    JumpIfTrueTrueOrPop(u32),

    PushOneOf,
    SetOneValid,
    PopOneOf,

    Call(u32),
}

impl core::fmt::Debug for Instruction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Instruction::TypeNull => f.write_str("TYPE_NULL"),
            Instruction::TypeBoolean => f.write_str("TYPE_BOOLEAN"),
            Instruction::TypeNumber => f.write_str("TYPE_NUMBER"),
            Instruction::TypeInteger => f.write_str("TYPE_INTEGER"),
            Instruction::TypeString => f.write_str("TYPE_STRING"),
            Instruction::TypeObject => f.write_str("TYPE_OBJECT"),
            Instruction::TypeArray => f.write_str("TYPE_ARRAY"),
            Instruction::TypeSet(types) => write!(f, "TYPE_SET {types:?}"),
            Instruction::MinimumU64(minimum) => write!(f, "MINIMUM_U64 {}", minimum.limit),
            Instruction::MinimumI64(minimum) => write!(f, "MINIMUM_I64 {}", minimum.limit),
            Instruction::MinimumF64(minimum) => write!(f, "MINIMUM_F64 {}", minimum.limit),
            Instruction::MaximumU64(maximum) => write!(f, "MAXIMUM_U64 {}", maximum.limit),
            Instruction::MaximumI64(maximum) => write!(f, "MAXIMUM_I64 {}", maximum.limit),
            Instruction::MaximumF64(maximum) => write!(f, "MAXIMUM_F64 {}", maximum.limit),
            Instruction::ExclusiveMinimumU64(minimum) => {
                write!(f, "EXCLUSIVE_MINIMUM_U64 {}", minimum.limit)
            }
            Instruction::ExclusiveMinimumI64(minimum) => {
                write!(f, "EXCLUSIVE_MINIMUM_I64 {}", minimum.limit)
            }
            Instruction::ExclusiveMinimumF64(minimum) => {
                write!(f, "EXCLUSIVE_MINIMUM_F64 {}", minimum.limit)
            }
            Instruction::ExclusiveMaximumU64(maximum) => {
                write!(f, "EXCLUSIVE_MAXIMUM_U64 {}", maximum.limit)
            }
            Instruction::ExclusiveMaximumI64(maximum) => {
                write!(f, "EXCLUSIVE_MAXIMUM_I64 {}", maximum.limit)
            }
            Instruction::ExclusiveMaximumF64(maximum) => {
                write!(f, "EXCLUSIVE_MAXIMUM_F64 {}", maximum.limit)
            }
            Instruction::MultipleOfFloat(multiple) => {
                write!(f, "MULTIPLE_OF_FLOAT {}", multiple.value)
            }
            Instruction::MultipleOfInteger(multiple) => {
                write!(f, "MULTIPLE_OF_INTEGER {}", multiple.value)
            }
            Instruction::True => f.write_str("TRUE"),
            Instruction::False => f.write_str("FALSE"),
            Instruction::JumpIfFalseOrPop(target) => write!(f, "JUMP_IF_FALSE_OR_POP {target}"),
            Instruction::JumpIfTrueOrPop(target) => write!(f, "JUMP_IF_TRUE_OR_POP {target}"),
            Instruction::JumpIfTrueTrueOrPop(target) => {
                write!(f, "JUMP_IF_TRUE_TRUE_OR_POP {target}")
            }
            Instruction::PushOneOf => f.write_str("PUSH_ONE_OF"),
            Instruction::SetOneValid => f.write_str("SET_ONE_VALID"),
            Instruction::PopOneOf => f.write_str("POP_ONE_OF"),
            Instruction::Call(pc) => write!(f, "CALL {pc}"),
        }
    }
}

macro_rules! define_min_max {
    ($($fn_name:ident => ($struct_name:ident, $instr_u64:ident, $instr_i64:ident, $instr_f64:ident)),* $(,)?) => {
        $(
            pub(crate) fn $fn_name(value: numeric::NumericValue) -> Self {
                match value {
                    numeric::NumericValue::U64(limit) => Instruction::$instr_u64(numeric::$struct_name::new(limit)),
                    numeric::NumericValue::I64(limit) => Instruction::$instr_i64(numeric::$struct_name::new(limit)),
                    numeric::NumericValue::F64(limit) => Instruction::$instr_f64(numeric::$struct_name::new(limit)),
                }
            }
        )*
    };
}

impl Instruction {
    define_min_max!(
        minimum => (Minimum, MinimumU64, MinimumI64, MinimumF64),
        maximum => (Maximum, MaximumU64, MaximumI64, MaximumF64),
        exclusive_minimum => (ExclusiveMinimum, ExclusiveMinimumU64, ExclusiveMinimumI64, ExclusiveMinimumF64),
        exclusive_maximum => (ExclusiveMaximum, ExclusiveMaximumU64, ExclusiveMaximumI64, ExclusiveMaximumF64),
    );
    pub(crate) fn multiple_of(value: numeric::NumericValue) -> Self {
        let value = value.as_f64();
        if value.fract() == 0. {
            Instruction::MultipleOfInteger(numeric::MultipleOfInteger::new(value))
        } else {
            Instruction::MultipleOfFloat(numeric::MultipleOfFloat::new(value))
        }
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct Instructions {
    pub(crate) instructions: Vec<Instruction>,
    pub(crate) locations: Vec<Location>,
}

impl Instructions {
    pub(crate) fn new() -> Self {
        Self {
            instructions: Vec::new(),
            locations: Vec::new(),
        }
    }
    /// Add a new `Instruction` without location information.
    pub(crate) fn add(&mut self, instr: Instruction) -> InstructionIdx {
        self.add_with_location(instr, Location::new())
    }

    /// Add a new `Instruction` with its location information.
    pub(crate) fn add_with_location(
        &mut self,
        instr: Instruction,
        loc: Location,
    ) -> InstructionIdx {
        let rv = self.instructions.len();
        self.instructions.push(instr);
        self.locations.push(loc);
        rv as InstructionIdx
    }

    /// Get an instruction by index.
    #[inline(always)]
    pub(crate) fn get(&self, idx: InstructionIdx) -> Option<&Instruction> {
        self.instructions.get(idx as usize)
    }

    /// Get an instruction by index mutably.
    #[inline(always)]
    pub(crate) fn get_mut(&mut self, idx: InstructionIdx) -> Option<&mut Instruction> {
        self.instructions.get_mut(idx as usize)
    }

    /// Number of instructions.
    #[inline(always)]
    pub(crate) fn len(&self) -> usize {
        self.instructions.len()
    }

    pub(crate) fn get_location(&self, idx: u32) -> Option<Location> {
        self.locations.get(idx as usize).cloned()
    }
}

impl core::fmt::Debug for Instructions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let max_loc_len = self
            .locations
            .iter()
            .map(|loc| loc.len())
            .max()
            .unwrap_or(0);

        struct Adapter<'a>(usize, usize, &'a Location, &'a Instruction);

        impl core::fmt::Debug for Adapter<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_fmt(format_args!(
                    "{:>05} | {:width$} | {:?}",
                    self.0,
                    self.2.as_str(),
                    self.3,
                    width = self.1
                ))
            }
        }

        let mut list = f.debug_list();

        for (idx, (loc, instr)) in self
            .locations
            .iter()
            .zip(self.instructions.iter())
            .enumerate()
        {
            list.entry(&Adapter(idx, max_loc_len, loc, instr));
        }
        list.finish()
    }
}

const _: () = const {
    assert!(std::mem::size_of::<Instruction>() <= 24);
};
