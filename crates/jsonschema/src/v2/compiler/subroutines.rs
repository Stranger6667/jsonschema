use ahash::{AHashMap, AHashSet};
use serde_json::Value;

use super::instructions::Instructions;

pub(super) type SubroutineId = u32;

#[derive(Clone)]
pub struct Subroutines {
    map: AHashMap<String, SubroutineId>,
    data: AHashMap<SubroutineId, Instructions>,
    in_progress: AHashSet<SubroutineId>,
}

impl Subroutines {
    pub(crate) fn new() -> Self {
        Self {
            map: AHashMap::new(),
            data: AHashMap::new(),
            in_progress: AHashSet::new(),
        }
    }

    pub(crate) fn get(&self, reference: &str) -> Option<SubroutineId> {
        self.map.get(reference).copied()
    }

    pub(crate) fn set_in_progress(&mut self, id: SubroutineId) {
        assert!(self.in_progress.insert(id));
    }
    pub(crate) fn unset_in_progress(&mut self, id: SubroutineId) {
        assert!(self.in_progress.remove(&id));
    }

    pub(crate) fn get_next_id(&mut self, reference: &str) -> u32 {
        let id = self.map.len() as SubroutineId;
        self.map.insert(reference.to_string(), id);
        id
    }
}

#[derive(Clone)]
pub struct Subroutine {
    pub(crate) instructions: Instructions,
    pub(crate) constants: Vec<Value>,
}
