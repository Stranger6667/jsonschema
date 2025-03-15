use crate::paths::Location;
use crate::v2::compiler::instructions::Instruction;
use std::fmt::Write;

#[derive(Debug, Clone)]
pub(super) struct EvaluationTracker {
    instructions: Vec<Instruction>,
    locations: Vec<Location>,
}

impl EvaluationTracker {
    pub(super) fn new() -> EvaluationTracker {
        EvaluationTracker {
            instructions: Vec::new(),
            locations: Vec::new(),
        }
    }

    pub(super) fn reset(&mut self) {
        self.instructions.clear();
        self.locations.clear();
    }

    pub(super) fn track(&mut self, instruction: &Instruction, location: Location) {
        self.instructions.push(*instruction);
        self.locations.push(location);
    }

    pub(super) fn report(&self) {
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

        let mut buf = String::from("Instructions:\n");
        for (idx, (loc, instr)) in self
            .locations
            .iter()
            .zip(self.instructions.iter())
            .enumerate()
        {
            writeln!(buf, "{:?}", &Adapter(idx, max_loc_len, loc, instr)).unwrap();
        }
        write!(buf, "Total Iterations: {}", self.instructions.len()).unwrap();
        println!("{buf}");
    }
}
