use crate::paths::Location;

use super::numeric;

pub(super) type InstructionIdx = u32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Instruction {
    TypeNumber {
        prefetch: numeric::PrefetchInfo,
        data: numeric::InlineData2x,
    },
    TypeInteger {
        prefetch: numeric::PrefetchInfo,
        data: numeric::InlineData2x,
    },
    MinimumU64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::Minimum<u64>,
        data: numeric::InlineData1x,
    },
    MinimumI64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::Minimum<i64>,
        data: numeric::InlineData1x,
    },
    MinimumF64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::Minimum<f64>,
        data: numeric::InlineData1x,
    },
    MaximumU64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::Maximum<u64>,
        data: numeric::InlineData1x,
    },
    MaximumI64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::Maximum<i64>,
        data: numeric::InlineData1x,
    },
    MaximumF64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::Maximum<f64>,
        data: numeric::InlineData1x,
    },
    ExclusiveMinimumU64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::ExclusiveMinimum<u64>,
        data: numeric::InlineData1x,
    },
    ExclusiveMinimumI64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::ExclusiveMinimum<i64>,
        data: numeric::InlineData1x,
    },
    ExclusiveMinimumF64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::ExclusiveMinimum<f64>,
        data: numeric::InlineData1x,
    },
    ExclusiveMaximumU64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::ExclusiveMaximum<u64>,
        data: numeric::InlineData1x,
    },
    ExclusiveMaximumI64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::ExclusiveMaximum<i64>,
        data: numeric::InlineData1x,
    },
    ExclusiveMaximumF64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::ExclusiveMaximum<f64>,
        data: numeric::InlineData1x,
    },
    MultipleOfFloat {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::MultipleOfFloat,
        data: numeric::InlineData1x,
    },
    MultipleOfInteger {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::MultipleOfInteger,
        data: numeric::InlineData1x,
    },
}

macro_rules! define_min_max {
    ($($fn_name:ident => ($struct_name:ident, $instr_u64:ident, $instr_i64:ident, $instr_f64:ident)),* $(,)?) => {
        $(
            pub(crate) fn $fn_name(
                prefetch: numeric::PrefetchInfo,
                value: numeric::NumericValue,
                data: numeric::InlineData1x,
            ) -> Self {
                match value {
                    numeric::NumericValue::U64(limit) => Instruction::$instr_u64 {
                        prefetch,
                        inner: numeric::$struct_name::new(limit),
                        data,
                    },
                    numeric::NumericValue::I64(limit) => Instruction::$instr_i64 {
                        prefetch,
                        inner: numeric::$struct_name::new(limit),
                        data,
                    },
                    numeric::NumericValue::F64(limit) => Instruction::$instr_f64 {
                        prefetch,
                        inner: numeric::$struct_name::new(limit),
                        data,
                    },
                }
            }
        )*
    };
}

impl Instruction {
    pub(crate) fn type_number(
        prefetch: numeric::PrefetchInfo,
        data: numeric::InlineData2x,
    ) -> Self {
        Instruction::TypeNumber { prefetch, data }
    }
    pub(crate) fn type_integer(
        prefetch: numeric::PrefetchInfo,
        data: numeric::InlineData2x,
    ) -> Self {
        Instruction::TypeInteger { prefetch, data }
    }

    define_min_max!(
        minimum => (Minimum, MinimumU64, MinimumI64, MinimumF64),
        maximum => (Maximum, MaximumU64, MaximumI64, MaximumF64),
        exclusive_minimum => (ExclusiveMinimum, ExclusiveMinimumU64, ExclusiveMinimumI64, ExclusiveMinimumF64),
        exclusive_maximum => (ExclusiveMaximum, ExclusiveMaximumU64, ExclusiveMaximumI64, ExclusiveMaximumF64),
    );
    pub(crate) fn multiple_of(
        prefetch: numeric::PrefetchInfo,
        value: numeric::NumericValue,
        data: numeric::InlineData1x,
    ) -> Self {
        let value = value.as_f64();
        if value.fract() == 0. {
            Instruction::MultipleOfInteger {
                prefetch,
                inner: numeric::MultipleOfInteger::new(value),
                data,
            }
        } else {
            Instruction::MultipleOfFloat {
                prefetch,
                inner: numeric::MultipleOfFloat::new(value),
                data,
            }
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

    /// Number of instructions.
    #[inline(always)]
    pub(crate) fn len(&self) -> usize {
        self.instructions.len()
    }

    pub(crate) fn get_location(&self, idx: u32) -> Option<Location> {
        self.locations.get(idx as usize).cloned()
    }
}

#[cfg(feature = "internal-debug")]
impl core::fmt::Debug for Instructions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        struct Adapter<'a>(usize, &'a Instruction);

        impl core::fmt::Debug for Adapter<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_fmt(format_args!("{:>05} | {:?}", self.0, self.1))
            }
        }

        let mut list = f.debug_list();
        for (idx, instr) in self.instructions.iter().enumerate() {
            list.entry(&Adapter(idx, instr));
        }
        list.finish()
    }
}

const _: () = const {
    assert!(std::mem::size_of::<Instruction>() == 24);
};
