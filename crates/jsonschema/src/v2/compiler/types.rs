use std::str::FromStr;

use serde_json::Value;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum JsonType {
    Array = 1 << 0,
    Boolean = 1 << 1,
    Integer = 1 << 2,
    Null = 1 << 3,
    Number = 1 << 4,
    Object = 1 << 5,
    String = 1 << 6,
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct JsonTypeSet(u8);

impl JsonTypeSet {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self(0)
    }

    pub(crate) fn from_value(value: &Value) -> Self {
        let mut set = JsonTypeSet::new();

        match value {
            Value::String(s) => {
                let json_type = s.parse::<JsonType>().expect("Invalid JSON type");
                set = set.insert(json_type);
            }
            Value::Array(arr) => {
                for item in arr {
                    if let Value::String(s) = item {
                        let json_type = s.parse::<JsonType>().expect("Invalid JSON type");
                        set = set.insert(json_type);
                    } else {
                        panic!("Expected all elements in the 'type' array to be strings");
                    }
                }
            }
            _ => panic!("Expected either a string or an array for the 'type' keyword"),
        }

        set
    }

    pub(crate) fn is_numeric_only(&self) -> bool {
        let numeric_mask = (JsonType::Integer as u8) | (JsonType::Number as u8);
        self.0 & !numeric_mask == 0 && (self.0 & numeric_mask != 0)
    }

    #[inline]
    pub(crate) const fn insert(mut self, ty: JsonType) -> Self {
        self.0 |= ty as u8;
        self
    }

    #[inline]
    pub(crate) const fn contains(self, ty: JsonType) -> bool {
        (self.0 & ty as u8) != 0
    }
}
