use core::fmt;
use std::str::FromStr;

use serde_json::Value;

use super::codegen::CodeGenerator;

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum JsonType {
    Array = 1 << 0,
    Boolean = 1 << 1,
    Integer = 1 << 2,
    Null = 1 << 3,
    Number = 1 << 4,
    Object = 1 << 5,
    String = 1 << 6,
}

impl fmt::Debug for JsonType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonType::Array => write!(f, "array"),
            JsonType::Boolean => write!(f, "boolean"),
            JsonType::Integer => write!(f, "integer"),
            JsonType::Null => write!(f, "null"),
            JsonType::Number => write!(f, "number"),
            JsonType::Object => write!(f, "object"),
            JsonType::String => write!(f, "string"),
        }
    }
}

impl JsonType {
    pub(crate) fn from_repr(repr: u8) -> Self {
        match repr {
            1 => JsonType::Array,
            2 => JsonType::Boolean,
            4 => JsonType::Integer,
            8 => JsonType::Null,
            16 => JsonType::Number,
            32 => JsonType::Object,
            64 => JsonType::String,
            _ => panic!("Invalid JsonType representation: {repr}"),
        }
    }
}

impl FromStr for JsonType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "array" => Ok(JsonType::Array),
            "boolean" => Ok(JsonType::Boolean),
            "integer" => Ok(JsonType::Integer),
            "null" => Ok(JsonType::Null),
            "number" => Ok(JsonType::Number),
            "object" => Ok(JsonType::Object),
            "string" => Ok(JsonType::String),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct JsonTypeSet(u8);

impl JsonTypeSet {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self(0)
    }

    #[inline]
    pub(crate) const fn insert(mut self, ty: JsonType) -> Self {
        self.0 |= ty as u8;
        self
    }

    #[inline]
    pub(crate) fn contains(self, value: &Value) -> bool {
        match value {
            Value::Array(_) => (self.0 & (JsonType::Array as u8)) != 0,
            Value::Bool(_) => (self.0 & (JsonType::Boolean as u8)) != 0,
            Value::Null => (self.0 & (JsonType::Null as u8)) != 0,
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    // Integer numbers match either Integer or Number types
                    (self.0 & (JsonType::Integer as u8)) != 0
                        || (self.0 & (JsonType::Number as u8)) != 0
                } else {
                    // Floating-point numbers only match Number type
                    (self.0 & (JsonType::Number as u8)) != 0
                }
            }
            Value::Object(_) => (self.0 & (JsonType::Object as u8)) != 0,
            Value::String(_) => (self.0 & (JsonType::String as u8)) != 0,
        }
    }

    #[inline]
    pub(crate) fn iter(&self) -> JsonTypeSetIterator {
        JsonTypeSetIterator { set: *self }
    }
}

impl fmt::Debug for JsonTypeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return Ok(());
        }

        write!(f, "(")?;

        let mut iter = self.iter();

        if let Some(ty) = iter.next() {
            write!(f, "{:?}", ty)?;
        }

        for ty in iter {
            write!(f, ", {:?}", ty)?;
        }

        write!(f, ")")
    }
}

#[derive(Debug)]
pub(crate) struct JsonTypeSetIterator {
    set: JsonTypeSet,
}

impl Iterator for JsonTypeSetIterator {
    type Item = JsonType;

    fn next(&mut self) -> Option<Self::Item> {
        if self.set.0 == 0 {
            None
        } else {
            // Find the least significant bit that is set
            let lsb = self.set.0 & -(self.set.0 as i8) as u8;

            // Clear the least significant bit
            self.set.0 &= self.set.0 - 1;

            // Convert the bit to PrimitiveType and return
            Some(JsonType::from_repr(lsb))
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let count = self.set.0.count_ones() as usize;
        (count, Some(count))
    }
}

pub(super) fn compile(codegen: &mut CodeGenerator, schema: &Value) {
    if let Some(types) = schema.get("type") {
        match types {
            Value::String(s) => {
                let ty = s.parse::<JsonType>().expect("Invalid JSON type");
                codegen.emit_type(ty);
            }
            Value::Array(arr) => {
                let mut set = JsonTypeSet::new();
                for item in arr {
                    if let Value::String(s) = item {
                        let json_type = s.parse::<JsonType>().expect("Invalid JSON type");
                        set = set.insert(json_type);
                    } else {
                        panic!("Expected all elements in the 'type' array to be strings");
                    }
                }
                codegen.emit_types(set);
            }
            _ => panic!("Expected either a string or an array for the 'type' keyword"),
        }
    };
}
