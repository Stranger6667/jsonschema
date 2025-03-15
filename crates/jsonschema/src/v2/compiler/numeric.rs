//! # Numeric Keywords Compiler and Memory Layout
//!
//! This module compiles numeric JSON Schema keywords—such as `minimum`, `maximum`,
//! `exclusiveMinimum`, `exclusiveMaximum`, and `multipleOf` into compact VM instructions
//! optimized for fast validation.
//!
//! ## Memory Layout (24 bytes per instruction)
//!
//! - 2 bytes: Instruction enum tag
//! - 2 bytes: Prefetch info
//! - 4 bytes: Extra prefetch info (reserved)
//! - 8 bytes: Inline value slot 0
//! - 8 bytes: Inline value slot 1
//!
//! ## Combined Prefetch and Inline Slots Strategy
//!
//! ### Prefetch Info
//!
//! The 16-bit prefetch field indicates which of the 5 possible numeric keywords
//! (minimum, maximum, exclusiveMinimum, exclusiveMaximum, multipleOf) are present
//! in the schema. Each keyword uses a 3-bit block encoding:
//! - Presence flag (bit2): Set to 1 if the keyword is present
//! - Numeric type (bits0-1): 00=u64, 01=i64, 10=f64
//!
//! ### Inline Value Slots
//!
//! The two 8-byte inline slots store values for the first two keywords (in a fixed order)
//! that were detected as present in the prefetch info. This approach:
//!
//! 1. Avoids executing additional instructions by storing values directly in
//!    the instruction stream
//! 2. Eliminates dispatch overhead by processing multiple validations at once
//! 3. Allows a single instruction to handle the common pattern of type + minimum + maximum
//!
//! This design yields ~25% performance improvement for numeric validation. Additional
//! keywords beyond the first two (rare in practice) require a separate instructions fetch.
use num_cmp::NumCmp;
use serde_json::{Number, Value};

use super::{
    super::ext::numeric,
    codegen::CodeGenerator,
    types::{JsonType, JsonTypeSet},
};

const INLINE_SLOTS: usize = 2;
pub(crate) type InlineData2x = [u64; INLINE_SLOTS];
pub(crate) type InlineData1x = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PrefetchInfo(u16);

impl PrefetchInfo {
    // Shifts for each keyword in the prefetch info
    pub(crate) const MINIMUM_SHIFT: u8 = 12;
    pub(crate) const MAXIMUM_SHIFT: u8 = 9;
    pub(crate) const EXCLUSIVE_MAXIMUM_SHIFT: u8 = 6;
    pub(crate) const EXCLUSIVE_MINIMUM_SHIFT: u8 = 3;
    pub(crate) const MULTIPLE_OF_SHIFT: u8 = 0;

    // Type constants
    pub(crate) const TYPE_U64: u8 = 0;
    pub(crate) const TYPE_I64: u8 = 1;
    pub(crate) const TYPE_F64: u8 = 2;

    pub(crate) fn new() -> PrefetchInfo {
        PrefetchInfo(0)
    }
    pub(crate) fn from_unchecked(inner: u16) -> PrefetchInfo {
        PrefetchInfo(inner)
    }
    fn is_empty(&self) -> bool {
        self.0 == 0
    }
    /// Check if a keyword is present in the prefetch info.
    #[inline(always)]
    pub(crate) fn has_keyword(&self, shift: u8) -> bool {
        ((self.0 >> shift) & 0b100) != 0
    }
    /// Get the numeric type for a keyword.
    #[inline(always)]
    pub(crate) fn get_type(&self, shift: u8) -> u8 {
        ((self.0 >> shift) & 0b11) as u8
    }
    /// Validate a numeric value against the prefetched minimum constraint.
    #[inline]
    pub(crate) fn is_valid_minimum(&self, value: &Number, limit: u64) -> bool {
        if !self.has_keyword(Self::MINIMUM_SHIFT) {
            return true;
        }

        match self.get_type(Self::MINIMUM_SHIFT) {
            Self::TYPE_U64 => numeric::ge(value, limit),
            Self::TYPE_I64 => numeric::ge(value, limit as i64),
            Self::TYPE_F64 => numeric::ge(value, f64::from_bits(limit)),
            _ => unreachable!(),
        }
    }
    /// Validate a numeric value against the prefetched maximum constraint.
    #[inline]
    pub(crate) fn is_valid_maximum(&self, value: &Number, limit: u64) -> bool {
        if !self.has_keyword(Self::MAXIMUM_SHIFT) {
            return true;
        }

        match self.get_type(Self::MAXIMUM_SHIFT) {
            Self::TYPE_U64 => numeric::le(value, limit),
            Self::TYPE_I64 => numeric::le(value, limit as i64),
            Self::TYPE_F64 => numeric::le(value, f64::from_bits(limit)),
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum NumericValue {
    U64(u64),
    I64(i64),
    F64(f64),
}

impl NumericValue {
    pub(crate) fn as_f64(self) -> f64 {
        match self {
            NumericValue::U64(u) => u as f64,
            NumericValue::I64(i) => i as f64,
            NumericValue::F64(f) => f,
        }
    }
    pub(crate) fn as_u64(self) -> u64 {
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
    let mut slots: [u64; INLINE_SLOTS] = [0, 0];
    let mut slot_count = 0;

    let mut process_keyword = |keyword: &str,
                               shift: u8,
                               encoder: fn(&NumericValue) -> u8|
     -> Option<(NumericValue, Option<usize>)> {
        if let Some(Value::Number(number)) = schema.get(keyword) {
            let number = parse_number(number);
            let block = encoder(&number);
            prefetch |= (block as u16) << shift;

            let slot = if slot_count < INLINE_SLOTS {
                slots[slot_count] = number.as_u64();
                let slot = slot_count;
                slot_count += 1;
                Some(slot)
            } else {
                None
            };

            Some((number, slot))
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
        if types.contains(JsonType::Number) {
            codegen.emit_number_type(prefetch, slots);
        } else {
            codegen.emit_integer_type(prefetch, slots);
        }
    }

    macro_rules! emit_numeric {
        ($( $keyword:ident => $emit_fn:ident ),* $(,)?) => {
            $(
                if let Some((value, inline_slot)) = $keyword {
                    let inline = inline_slot
                        .filter(|&idx| idx + 1 < slot_count)
                        .map(|idx| slots[idx + 1])
                        .unwrap_or(0);
                    codegen.$emit_fn(prefetch, value, inline);
                }
            )*
        };
    }

    emit_numeric!(
        minimum => emit_minimum,
        maximum => emit_maximum,
        exclusive_max => emit_exclusive_maximum,
        exclusive_min => emit_exclusive_minimum,
        multiple_of => emit_multiple_of,
    );
}

macro_rules! define_numeric_keywords {
    ($($struct_name:ident => $fn_name:path),* $(,)?) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq)]
            pub(crate) struct $struct_name<T> {
                pub(super) limit: T,
            }

            impl<T> $struct_name<T>
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
                    $fn_name(value, self.limit)
                }
            }
        )*
    };
}

define_numeric_keywords!(
    Minimum => numeric::ge,
    Maximum => numeric::le,
    ExclusiveMinimum => numeric::gt,
    ExclusiveMaximum => numeric::lt,
);

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MultipleOfFloat {
    pub(super) value: f64,
}

impl MultipleOfFloat {
    pub(crate) fn new(value: f64) -> Self {
        Self { value }
    }

    pub(crate) fn is_valid(&self, value: &Number) -> bool {
        numeric::is_multiple_of_float(value, self.value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MultipleOfInteger {
    pub(super) value: f64,
}

impl MultipleOfInteger {
    pub(crate) fn new(value: f64) -> Self {
        Self { value }
    }

    pub(crate) fn is_valid(&self, value: &Number) -> bool {
        numeric::is_multiple_of_integer(value, self.value)
    }
}
