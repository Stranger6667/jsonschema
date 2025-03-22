use std::hash::{Hash, Hasher};

#[derive(Debug, Copy, Clone)]
pub enum Number {
    PositiveInteger(u64),
    NegativeInteger(i64),
    Float(f64),
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
