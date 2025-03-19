use std::hash::{Hash, Hasher};

#[derive(Debug, Copy, Clone)]
pub enum Number {
    Positive(u64),
    Negative(i64),
    Float(f64),
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Number::Positive(a), Number::Positive(b)) => a == b,
            (Number::Negative(a), Number::Negative(b)) => a == b,
            (Number::Float(a), Number::Float(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Number {}

impl Hash for Number {
    fn hash<H: Hasher>(&self, h: &mut H) {
        match *self {
            Number::Positive(i) => i.hash(h),
            Number::Negative(i) => i.hash(h),
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
