use super::super::compiler::instructions::Instruction;

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

    pub(super) fn track(&mut self, instruction: &Instruction) {
        self.instructions.push(*instruction);
    }

    pub(super) fn report(&self) {
        println!("Total Iterations: {}", self.instructions.len());
    }
}
