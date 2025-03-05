use num_cmp::NumCmp;
use serde_json::{Map, Number, Value};

use crate::{
    compiler,
    keywords::CompilationResult,
    vm::{Instruction, NodeMetadata},
};

#[derive(Debug, Clone)]
pub(crate) struct Maximum<T> {
    limit: T,
}

impl<T> Maximum<T>
where
    T: Copy,
    u64: NumCmp<T>,
    i64: NumCmp<T>,
    f64: NumCmp<T>,
{
    pub(crate) fn execute(&self, value: &Number) -> bool {
        if let Some(v) = value.as_u64() {
            return !NumCmp::num_gt(v, self.limit);
        }
        if let Some(v) = value.as_i64() {
            return !NumCmp::num_gt(v, self.limit);
        }
        let v = value.as_f64().expect("Always valid");
        !NumCmp::num_gt(v, self.limit)
    }

    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        _: &'a Map<String, Value>,
        schema: &'a Value,
    ) -> Option<CompilationResult<'a>> {
        if let Value::Number(limit) = schema {
            let location = ctx.location().join("maximum");
            let metadata = NodeMetadata {
                location,
                keyword_value: schema.clone(),
            };

            let instruction = if let Some(limit) = limit.as_u64() {
                Instruction::MaximumU64(Maximum { limit })
            } else if let Some(limit) = limit.as_i64() {
                Instruction::MaximumI64(Maximum { limit })
            } else {
                let limit = limit.as_f64().expect("Always valid");
                Instruction::MaximumF64(Maximum { limit })
            };

            ctx.push_instruction(instruction, metadata);
        }
        None
    }
}
