use crate::paths::Location;

use super::numeric;

pub(super) type InstructionIdx = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Instruction {
    TypeInteger {
        prefetch: numeric::PrefetchInfo,
        data: numeric::InlineData2x,
    },
    MinimumU64 {
        prefetch: numeric::PrefetchInfo,
        inner: numeric::Minimum<u64>,
        data: numeric::InlineData1x,
    },
}

impl Instruction {
    pub(crate) fn type_integer(
        prefetch: numeric::PrefetchInfo,
        data: numeric::InlineData2x,
    ) -> Self {
        Instruction::TypeInteger { prefetch, data }
    }
    pub(crate) fn minimum(
        prefetch: numeric::PrefetchInfo,
        value: numeric::NumericValue,
        data: numeric::InlineData1x,
    ) -> Self {
        match value {
            numeric::NumericValue::U64(limit) => Instruction::MinimumU64 {
                prefetch,
                inner: numeric::Minimum::new(limit),
                data,
            },
            numeric::NumericValue::I64(i) => todo!(),
            numeric::NumericValue::F64(f) => todo!(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
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
