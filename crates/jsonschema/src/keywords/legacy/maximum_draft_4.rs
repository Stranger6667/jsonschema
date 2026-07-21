use crate::{
    compiler,
    keywords::{minmax, CompilationResult},
    Json,
};
use serde_json::{Map, Value};

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    if let Some(Value::Bool(true)) = parent.get("exclusiveMaximum") {
        minmax::compile_exclusive_maximum(ctx, parent, schema)
    } else {
        minmax::compile_maximum(ctx, parent, schema)
    }
}
