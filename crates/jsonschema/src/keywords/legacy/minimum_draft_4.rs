use crate::{
    compiler,
    keywords::{legacy::Draft4ExclusiveValidator, minmax, BoxedValidator, CompilationResult},
    Json,
};
use serde_json::{Map, Value};

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    if let Some(Value::Bool(true)) = parent.get("exclusiveMinimum") {
        compile_exclusive(ctx, parent, schema)
    } else {
        minmax::compile_minimum(ctx, parent, schema)
    }
}

#[inline]
fn compile_exclusive<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    let inner = minmax::compile_exclusive_minimum(ctx, parent, schema)?;
    Some(inner.map(|inner| {
        Box::new(Draft4ExclusiveValidator::new(
            inner,
            ctx.location().join("minimum"),
        )) as BoxedValidator<F>
    }))
}
