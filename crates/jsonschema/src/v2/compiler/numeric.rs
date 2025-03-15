use num_cmp::NumCmp;
use serde_json::{Number, Value};

use super::{super::ext::numeric, codegen::CodeGenerator};

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

pub(super) fn compile(codegen: &mut CodeGenerator, schema: &Value) {
    macro_rules! emit_numeric {
        ($( $keyword:expr => $emit_fn:ident ),* $(,)?) => {
            $(
                {
                    if let Some(Value::Number(number)) = schema.get($keyword) {
                        let value = parse_number(number);
                        codegen.$emit_fn(value);
                    }
                }
            )*
        };
    }

    emit_numeric!(
        "minimum" => emit_minimum,
        "maximum" => emit_maximum,
        "exclusiveMaximum" => emit_exclusive_maximum,
        "exclusiveMinimum" => emit_exclusive_minimum,
        "multipleOf" => emit_multiple_of,
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

                #[inline(always)]
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
