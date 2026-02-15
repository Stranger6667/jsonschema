use serde_json::Number;

use super::numeric;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BoundOp {
    Lt,
    Lte,
    Gt,
    Gte,
}

impl BoundOp {
    pub(crate) fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Lt),
            1 => Some(Self::Lte),
            2 => Some(Self::Gt),
            3 => Some(Self::Gte),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CompiledBound {
    U64 {
        op: BoundOp,
        limit: u64,
    },
    I64 {
        op: BoundOp,
        limit: i64,
    },
    F64 {
        op: BoundOp,
        limit: f64,
    },
    #[cfg(feature = "arbitrary-precision")]
    BigInt {
        op: BoundOp,
        limit: num_bigint::BigInt,
    },
    #[cfg(feature = "arbitrary-precision")]
    BigFrac {
        op: BoundOp,
        limit: fraction::BigFraction,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CompiledMultipleOf {
    Integer(f64),
    Float(f64),
    #[cfg(feature = "arbitrary-precision")]
    BigInt(num_bigint::BigInt),
    #[cfg(feature = "arbitrary-precision")]
    BigFrac(fraction::BigFraction),
    Unsupported,
}

#[inline]
fn check_primitive_bound<T>(op: BoundOp, value: &Number, limit: T) -> bool
where
    T: Copy + num_traits::ToPrimitive,
    u64: num_cmp::NumCmp<T>,
    i64: num_cmp::NumCmp<T>,
    f64: num_cmp::NumCmp<T>,
{
    match op {
        BoundOp::Lt => numeric::lt(value, limit),
        BoundOp::Lte => numeric::le(value, limit),
        BoundOp::Gt => numeric::gt(value, limit),
        BoundOp::Gte => numeric::ge(value, limit),
    }
}

#[cfg(feature = "arbitrary-precision")]
#[inline]
fn infinity_cmp(op: BoundOp, is_negative: bool) -> bool {
    match op {
        BoundOp::Gte | BoundOp::Gt => !is_negative,
        BoundOp::Lte | BoundOp::Lt => is_negative,
    }
}

#[cfg(feature = "arbitrary-precision")]
#[inline]
fn check_bigint_bound(op: BoundOp, limit: &num_bigint::BigInt, value: &Number) -> bool {
    use fraction::BigFraction;

    if let Some(instance_bigint) = numeric::bignum::try_parse_bigint(value) {
        return match op {
            BoundOp::Lt => instance_bigint < *limit,
            BoundOp::Lte => instance_bigint <= *limit,
            BoundOp::Gt => instance_bigint > *limit,
            BoundOp::Gte => instance_bigint >= *limit,
        };
    }

    if let Some(v) = value.as_u64() {
        return match op {
            BoundOp::Lt => numeric::bignum::u64_lt_bigint(v, limit),
            BoundOp::Lte => numeric::bignum::u64_le_bigint(v, limit),
            BoundOp::Gt => numeric::bignum::u64_gt_bigint(v, limit),
            BoundOp::Gte => numeric::bignum::u64_ge_bigint(v, limit),
        };
    }

    if let Some(v) = value.as_i64() {
        return match op {
            BoundOp::Lt => numeric::bignum::i64_lt_bigint(v, limit),
            BoundOp::Lte => numeric::bignum::i64_le_bigint(v, limit),
            BoundOp::Gt => numeric::bignum::i64_gt_bigint(v, limit),
            BoundOp::Gte => numeric::bignum::i64_ge_bigint(v, limit),
        };
    }

    if let Some(v) = value.as_f64() {
        return match op {
            BoundOp::Lt => numeric::bignum::f64_lt_bigint(v, limit),
            BoundOp::Lte => numeric::bignum::f64_le_bigint(v, limit),
            BoundOp::Gt => numeric::bignum::f64_gt_bigint(v, limit),
            BoundOp::Gte => numeric::bignum::f64_ge_bigint(v, limit),
        };
    }

    if let Some(instance_bigfrac) = numeric::bignum::try_parse_bigfraction(value) {
        let limit_frac = BigFraction::from(limit.clone());
        return match op {
            BoundOp::Lt => instance_bigfrac < limit_frac,
            BoundOp::Lte => instance_bigfrac <= limit_frac,
            BoundOp::Gt => instance_bigfrac > limit_frac,
            BoundOp::Gte => instance_bigfrac >= limit_frac,
        };
    }

    infinity_cmp(op, value.as_str().starts_with('-'))
}

#[cfg(feature = "arbitrary-precision")]
#[inline]
fn check_bigfrac_bound(op: BoundOp, limit: &fraction::BigFraction, value: &Number) -> bool {
    if let Some(instance_bigfrac) = numeric::bignum::try_parse_bigfraction(value) {
        return match op {
            BoundOp::Lt => instance_bigfrac < *limit,
            BoundOp::Lte => instance_bigfrac <= *limit,
            BoundOp::Gt => instance_bigfrac > *limit,
            BoundOp::Gte => instance_bigfrac >= *limit,
        };
    }

    if let Some(v) = value.as_u64() {
        return match op {
            BoundOp::Lt => numeric::bignum::u64_lt_bigfrac(v, limit),
            BoundOp::Lte => numeric::bignum::u64_le_bigfrac(v, limit),
            BoundOp::Gt => numeric::bignum::u64_gt_bigfrac(v, limit),
            BoundOp::Gte => numeric::bignum::u64_ge_bigfrac(v, limit),
        };
    }

    if let Some(v) = value.as_i64() {
        return match op {
            BoundOp::Lt => numeric::bignum::i64_lt_bigfrac(v, limit),
            BoundOp::Lte => numeric::bignum::i64_le_bigfrac(v, limit),
            BoundOp::Gt => numeric::bignum::i64_gt_bigfrac(v, limit),
            BoundOp::Gte => numeric::bignum::i64_ge_bigfrac(v, limit),
        };
    }

    if let Some(v) = value.as_f64() {
        return match op {
            BoundOp::Lt => numeric::bignum::f64_lt_bigfrac(v, limit),
            BoundOp::Lte => numeric::bignum::f64_le_bigfrac(v, limit),
            BoundOp::Gt => numeric::bignum::f64_gt_bigfrac(v, limit),
            BoundOp::Gte => numeric::bignum::f64_ge_bigfrac(v, limit),
        };
    }

    // Dynamic BigFraction validators treat this branch as valid because
    // extremely large scientific notation cannot be compared reliably.
    true
}

pub(crate) fn compile_bound(op: BoundOp, limit: &Number) -> CompiledBound {
    if let Some(value) = limit.as_u64() {
        return CompiledBound::U64 { op, limit: value };
    }
    if let Some(value) = limit.as_i64() {
        return CompiledBound::I64 { op, limit: value };
    }

    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(value) = numeric::bignum::try_parse_bigint(limit) {
            return CompiledBound::BigInt { op, limit: value };
        }
        if let Some(value) = numeric::bignum::try_parse_bigfraction(limit) {
            return CompiledBound::BigFrac { op, limit: value };
        }
    }

    if let Some(value) = limit.as_f64() {
        return CompiledBound::F64 { op, limit: value };
    }

    #[cfg(feature = "arbitrary-precision")]
    {
        let limit = if limit.as_str().starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
        return CompiledBound::F64 { op, limit };
    }

    #[cfg(not(feature = "arbitrary-precision"))]
    {
        unreachable!("non-AP serde_json::Number always has an f64 representation");
    }
}

pub(crate) fn check_bound(compiled: &CompiledBound, value: &Number) -> bool {
    match compiled {
        CompiledBound::U64 { op, limit } => check_primitive_bound(*op, value, *limit),
        CompiledBound::I64 { op, limit } => check_primitive_bound(*op, value, *limit),
        CompiledBound::F64 { op, limit } => check_primitive_bound(*op, value, *limit),
        #[cfg(feature = "arbitrary-precision")]
        CompiledBound::BigInt { op, limit } => check_bigint_bound(*op, limit, value),
        #[cfg(feature = "arbitrary-precision")]
        CompiledBound::BigFrac { op, limit } => check_bigfrac_bound(*op, limit, value),
    }
}

pub(crate) fn compile_multiple_of(multiple_of: &Number) -> CompiledMultipleOf {
    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(value) = numeric::bignum::try_parse_bigint(multiple_of) {
            return CompiledMultipleOf::BigInt(value);
        }
        if let Some(value) = numeric::bignum::try_parse_bigfraction(multiple_of) {
            return CompiledMultipleOf::BigFrac(value);
        }
    }

    if let Some(value) = multiple_of.as_f64() {
        return if value.fract() == 0.0 {
            CompiledMultipleOf::Integer(value)
        } else {
            CompiledMultipleOf::Float(value)
        };
    }

    CompiledMultipleOf::Unsupported
}

pub(crate) fn check_multiple_of(compiled: &CompiledMultipleOf, value: &Number) -> bool {
    match compiled {
        CompiledMultipleOf::Integer(multiple) => numeric::is_multiple_of_integer(value, *multiple),
        CompiledMultipleOf::Float(multiple) => numeric::is_multiple_of_float(value, *multiple),
        #[cfg(feature = "arbitrary-precision")]
        CompiledMultipleOf::BigInt(multiple) => {
            use num_bigint::BigInt;
            use num_traits::One;

            if let Some(instance_bigint) = numeric::bignum::try_parse_bigint(value) {
                return numeric::bignum::is_multiple_of_bigint(&instance_bigint, multiple);
            }

            if let Some(v) = value.as_u64() {
                let v_bigint = BigInt::from(v);
                return numeric::bignum::is_multiple_of_bigint(&v_bigint, multiple);
            }

            if let Some(v) = value.as_i64() {
                let v_bigint = BigInt::from(v);
                return numeric::bignum::is_multiple_of_bigint(&v_bigint, multiple);
            }

            if let Some(v) = value.as_f64() {
                if v.fract() != 0.0 {
                    return false;
                }
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                if v.is_finite() && v.abs() < (1u64 << 53) as f64 {
                    let v_bigint = BigInt::from(v as i64);
                    return numeric::bignum::is_multiple_of_bigint(&v_bigint, multiple);
                }
                return false;
            }

            if let Some(instance_bigfrac) = numeric::bignum::try_parse_bigfraction(value) {
                if instance_bigfrac.denom().is_none_or(One::is_one) {
                    if let Some(numer) = instance_bigfrac.numer() {
                        let instance_bigint = BigInt::from(numer.clone());
                        return numeric::bignum::is_multiple_of_bigint(&instance_bigint, multiple);
                    }
                }
            }

            false
        }
        #[cfg(feature = "arbitrary-precision")]
        CompiledMultipleOf::BigFrac(multiple) => {
            use num_traits::ToPrimitive;

            if let Some(instance_bigfrac) = numeric::bignum::try_parse_bigfraction(value) {
                return numeric::bignum::is_multiple_of_bigfrac(&instance_bigfrac, multiple);
            }

            if let Some(instance_bigint) = numeric::bignum::try_parse_bigint(value) {
                let value_frac = fraction::BigFraction::from(instance_bigint);
                return numeric::bignum::is_multiple_of_bigfrac(&value_frac, multiple);
            }

            if let Some(v) = value.as_u64() {
                let value_frac = fraction::BigFraction::from(v);
                return numeric::bignum::is_multiple_of_bigfrac(&value_frac, multiple);
            }

            if let Some(v) = value.as_i64() {
                let value_frac = fraction::BigFraction::from(v);
                return numeric::bignum::is_multiple_of_bigfrac(&value_frac, multiple);
            }

            let multiple_f64 = multiple.to_f64().unwrap_or(f64::INFINITY);
            numeric::is_multiple_of_float(value, multiple_f64)
        }
        CompiledMultipleOf::Unsupported => true,
    }
}
