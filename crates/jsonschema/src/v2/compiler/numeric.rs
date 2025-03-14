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
use num_cmp::NumCmp;
use serde_json::{Number, Value};

use super::{
    codegen::CodeGenerator,
    types::{JsonType, JsonTypeSet},
};

pub(crate) type InlineData2x = [u64; 2];
pub(crate) type InlineData1x = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PrefetchInfo(u16);

impl PrefetchInfo {
    pub(crate) fn new() -> PrefetchInfo {
        PrefetchInfo(0)
    }
    pub(crate) fn from_unchecked(inner: u16) -> PrefetchInfo {
        PrefetchInfo(inner)
    }
    fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum NumericValue {
    U64(u64),
    I64(i64),
    F64(f64),
}

impl NumericValue {
    fn as_u64(self) -> u64 {
        match self {
            NumericValue::U64(u) => u,
            NumericValue::I64(i) => i as u64,
            NumericValue::F64(f) => f as u64,
        }
    }
}

impl From<u64> for NumericValue {
    fn from(value: u64) -> Self {
        NumericValue::U64(value)
    }
}

impl From<i64> for NumericValue {
    fn from(value: i64) -> Self {
        NumericValue::I64(value)
    }
}

impl From<f64> for NumericValue {
    fn from(value: f64) -> Self {
        NumericValue::F64(value)
    }
}

fn parse_number(val: &Number) -> NumericValue {
    if let Some(u) = val.as_u64() {
        NumericValue::U64(u)
    } else if let Some(i) = val.as_i64() {
        NumericValue::I64(i)
    } else if let Some(f) = val.as_f64() {
        NumericValue::F64(f)
    } else {
        panic!("Invalid numeric value in schema");
    }
}

/// For keywords `minimum`, `maximum`, `exclusiveMaximum`, and `exclusiveMinimum`,
/// encode a 3-bit block:
///   - Bit2: presence flag (set to 1)
///   - Bits0-1: numeric type (00: u64, 01: i64, 10: f64)
fn encode_block(num: &NumericValue) -> u8 {
    match num {
        NumericValue::U64(_) => 0b100,
        NumericValue::I64(_) => 0b101,
        NumericValue::F64(_) => 0b110,
    }
}

/// For the `multipleOf` keyword, we use a similar encoding where:
///   - Bit2: presence flag (set to 1)
///   - Bits0-1: kind of multiple (00: integer, 01: float)
fn encode_multiple_of(num: &NumericValue) -> u8 {
    match num {
        NumericValue::F64(_) => 0b101,
        _ => 0b100, // Treat U64 and I64 as integer multiples.
    }
}

pub(super) fn compile(codegen: &mut CodeGenerator, types: JsonTypeSet, schema: &Value) {
    // We'll pack our metadata in a 16-bit field.
    let mut prefetch: u16 = 0;
    // Reserve two inline slots for numeric values.
    let mut inline_slots: [u64; 2] = [0, 0];
    let mut inline_count = 0;

    let mut process_keyword = |keyword: &str,
                               shift: u8,
                               encoder: fn(&NumericValue) -> u8|
     -> Option<(NumericValue, Option<usize>)> {
        if let Some(Value::Number(number)) = schema.get(keyword) {
            let number = parse_number(number);
            let block = encoder(&number);
            prefetch |= (block as u16) << shift;
            let inline_slot = if inline_count < 2 {
                inline_slots[inline_count] = number.as_u64();
                let slot = inline_count;
                inline_count += 1;
                Some(slot)
            } else {
                None
            };
            Some((number, inline_slot))
        } else {
            None
        }
    };

    let minimum = process_keyword("minimum", 12, encode_block);
    let maximum = process_keyword("maximum", 9, encode_block);
    let exclusive_max = process_keyword("exclusiveMaximum", 6, encode_block);
    let exclusive_min = process_keyword("exclusiveMinimum", 3, encode_block);
    let multiple_of = process_keyword("multipleOf", 0, encode_multiple_of);

    let prefetch = PrefetchInfo(prefetch);

    // If the schema's type is exclusively numeric, emit the numeric type check
    // with the prefetch info and inline values for the following numeric instructions.
    if types.is_numeric_only() {
        if types.contains(JsonType::Integer) {
            codegen.emit_integer_type(prefetch, inline_slots);
        } else {
            //codegen.emit_integer_type(prefetch_info, inline_slots);
        }
    }

    // Emit individual numeric keyword instructions.
    if let Some((value, inline_slot)) = minimum {
        // TODO: use inline slot value here
        codegen.emit_minimum(prefetch, value, 0);
    }
    //if let Some((num, inline_slot)) = maximum {
    //    codegen.emit_maximum(prefetch, inline_slot, num);
    //}
    //if let Some((num, inline_slot)) = exclusive_max {
    //    codegen.emit_exclusive_maximum(prefetch, inline_slot, num);
    //}
    //if let Some((num, inline_slot)) = exclusive_min {
    //    codegen.emit_exclusive_minimum(prefetch, inline_slot, num);
    //}
    //if let Some((num, inline_slot)) = multiple_of {
    //    codegen.emit_multiple_of(prefetch, inline_slot, num);
    //}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Minimum<T> {
    pub(super) limit: T,
}

impl<T> Minimum<T>
where
    T: Copy,
    u64: NumCmp<T>,
    i64: NumCmp<T>,
    f64: NumCmp<T>,
{
    pub(crate) fn new(limit: T) -> Self {
        Self { limit }
    }
    pub(crate) fn is_valid(&self, value: &Number) -> bool {
        if let Some(v) = value.as_u64() {
            return !NumCmp::num_lt(v, self.limit);
        }
        if let Some(v) = value.as_i64() {
            return !NumCmp::num_lt(v, self.limit);
        }
        let v = value.as_f64().expect("Always valid");
        !NumCmp::num_lt(v, self.limit)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Maximum<T> {
    pub(super) limit: T,
}

impl<T> Maximum<T>
where
    T: Copy,
    u64: NumCmp<T>,
    i64: NumCmp<T>,
    f64: NumCmp<T>,
{
    pub(crate) fn new(limit: T) -> Self {
        Self { limit }
    }
    pub(crate) fn is_valid(&self, value: &Number) -> bool {
        if let Some(v) = value.as_u64() {
            return !NumCmp::num_gt(v, self.limit);
        }
        if let Some(v) = value.as_i64() {
            return !NumCmp::num_gt(v, self.limit);
        }
        let v = value.as_f64().expect("Always valid");
        !NumCmp::num_gt(v, self.limit)
    }
}
