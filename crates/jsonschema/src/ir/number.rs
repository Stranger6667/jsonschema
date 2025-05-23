use core::fmt;
use std::hash::{Hash, Hasher};

#[derive(Debug, Copy, Clone)]
pub(crate) enum Number {
    PositiveInteger(u64),
    NegativeInteger(i64),
    Float(f64),
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Number::PositiveInteger(n) => write!(f, "{}", n),
            Number::NegativeInteger(n) => write!(f, "{}", n),
            Number::Float(n) => write!(f, "{}", n),
        }
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Number::PositiveInteger(a), Number::PositiveInteger(b)) => a == b,
            (Number::NegativeInteger(a), Number::NegativeInteger(b)) => a == b,
            (Number::Float(a), Number::Float(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Number {}

impl Hash for Number {
    fn hash<H: Hasher>(&self, h: &mut H) {
        match *self {
            Number::PositiveInteger(i) => i.hash(h),
            Number::NegativeInteger(i) => i.hash(h),
            Number::Float(f) => {
                if f == 0.0f64 {
                    0.0f64.to_bits().hash(h);
                } else {
                    f.to_bits().hash(h);
                }
            }
        }
    }
}

impl From<&serde_json::Number> for Number {
    fn from(value: &serde_json::Number) -> Self {
        if let Some(u) = value.as_u64() {
            Number::PositiveInteger(u)
        } else if let Some(i) = value.as_i64() {
            Number::NegativeInteger(i)
        } else {
            Number::Float(value.as_f64().expect("Always succeeds"))
        }
    }
}
