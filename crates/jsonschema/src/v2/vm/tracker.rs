use super::super::compiler::instructions::Instruction;
use std::fmt::Write;

#[derive(Debug, Clone)]
pub(super) struct EvaluationTracker {
    instructions: Vec<Instruction>,
}

impl EvaluationTracker {
    pub(super) fn new() -> EvaluationTracker {
        EvaluationTracker {
            instructions: Vec::new(),
        }
    }

    pub(super) fn reset(&mut self) {
        self.instructions.clear();
    }

    pub(super) fn track(&mut self, instruction: &Instruction) {
        self.instructions.push(*instruction);
    }

    pub(super) fn report(&self) {
        struct Adapter<'a>(usize, &'a Instruction);

        impl core::fmt::Debug for Adapter<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_fmt(format_args!("{:>05} | {:?}", self.0, self.1))
            }
        }

        let mut buf = String::from("Instructions:\n");
        for (idx, instr) in self.instructions.iter().enumerate() {
            writeln!(buf, "{:?}", &Adapter(idx, instr)).unwrap();
        }
        write!(buf, "Total Iterations: {}", self.instructions.len()).unwrap();
        println!("{buf}");
    }
}
