mod numeric;
use serde_json::Value;

use crate::paths::Location;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PrefetchInfo(u16);

impl PrefetchInfo {
    pub(crate) fn new() -> PrefetchInfo {
        PrefetchInfo(0)
    }
    fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Instruction {
    TypeInteger {
        prefetch_info: PrefetchInfo,
        value0: usize,
        value1: usize,
    },
}
const _: () = const {
    assert!(std::mem::size_of::<Instruction>() == 24);
};

#[derive(Debug, Clone)]
pub(crate) struct Program {
    pub(crate) instructions: Vec<Instruction>,
    pub(crate) locations: Vec<Location>,
    pub(crate) constants: Vec<Value>,
}

impl Program {
    pub(super) fn compile(schema: &Value) -> Program {
        let mut ctx = CompilationContext::new();
        ctx.compile(schema);
        Program {
            instructions: ctx.instructions,
            locations: ctx.locations.recorded,
            constants: ctx.constants,
        }
    }
}

struct LocationContext {
    recorded: Vec<Location>,
    stack: Vec<Location>,
    top: Location,
}

impl LocationContext {
    fn new() -> Self {
        Self {
            recorded: Vec::new(),
            stack: Vec::new(),
            top: Location::new(),
        }
    }
    fn push(&mut self, key: &str) {
        let mut new = self.top.join(key);
        std::mem::swap(&mut self.top, &mut new);
        self.stack.push(new);
    }
    fn pop(&mut self) {
        let mut top = self.stack.pop().expect("Empty stack");
        std::mem::swap(&mut self.top, &mut top);
    }

    fn record(&mut self, segment: &str) {
        self.recorded.push(self.top.join(segment));
    }
}

struct CompilationContext {
    instructions: Vec<Instruction>,
    locations: LocationContext,
    constants: Vec<Value>,
}

impl CompilationContext {
    fn new() -> Self {
        Self {
            instructions: Vec::new(),
            locations: LocationContext::new(),
            constants: Vec::new(),
        }
    }

    fn compile(&mut self, schema: &Value) {
        let ty = schema.get("type");
        numeric::compile(self, schema);
    }

    fn emit_integer_type(&mut self, prefetch_info: PrefetchInfo) {
        self.instructions.push(Instruction::TypeInteger {
            prefetch_info,
            value0: 0,
            value1: 0,
        });
        self.locations.record("type");
    }
}
