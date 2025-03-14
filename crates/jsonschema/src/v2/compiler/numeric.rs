//! ### Numeric Keywords
//!
//! - `minimum`
//! - `maximum`
//! - `multipleOf`
//! - `exclusiveMaximum`
//! - `exclusiveMinimum`
//!
//! All have 8 bytes of useful data right now. Idea always encode what numeric keywords follow the current one. They should always be in the same order, so 1 byte could be used to fetch all numeric instructions. Assume now are 14 variations + future extension for bignums,
//!
//! ```
//! TYPE_INTEGER
//!
//! 5 keywords - 5 bits +  2 bits - 4 types per keyword (i64, u64, f64, bigint)
//!
//! 15 bits / 2 bytes
//!
//! 1 00
//!
//! if keyword is present + its operand type
//!
//! ```
//!
//! This way all numeric keywords will take just 1 iteration of the main loop.
//! Superinstruction like MinMax could simplify it. if we have
//!
//! **IDEA**: Encode information about how many instructions prefetch immediately into the current instruction. This way decoding of instructions could be simpler, without extra iterations of the main loop - all following numeric instructions will be known.
//!
//! 2 bytes - opcode
//! 2 bytes - prefetch info
//! 4 bytes - extra for the future extensions
//! 8 bytes - data 1 (e.g `minimum` value)
//! 8 bytes - data 2 (e.g. `maximum` value for MinMax superinstruction)
//!
//! Should the order of keywords matter? The last keyword may avoid any prefetching, so it should be the most popular one (`minimum`)
use serde_json::Value;

use super::{CompilationContext, PrefetchInfo};

pub(super) fn compile(ctx: &mut CompilationContext, schema: &Value) {
    // TODO: `type` is needed elsewhere, avoid multiple lookups
    match (
        schema.get("type"),
        schema.get("minimum"),
        schema.get("maximum"),
        schema.get("exclusiveMinimum"),
        schema.get("exclusiveMaximum"),
        schema.get("multipleOf"),
    ) {
        (
            Some(Value::String(ty)),
            Some(Value::Number(minimum)),
            Some(Value::Number(maximum)),
            Some(Value::Number(exclusive_minimum)),
            Some(Value::Number(exclusive_maximum)),
            Some(Value::Number(multiple_of)),
        ) if ty == "integer" => {
            dbg!(42);
        }
        (Some(Value::String(ty)), None, None, None, None, None) if ty == "integer" => {
            ctx.emit_integer_type(PrefetchInfo::new());
        }
        _ => {}
    }
}
