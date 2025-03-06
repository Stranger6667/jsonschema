use serde_json::{Map, Number, Value};

use crate::primitive_type::{PrimitiveType, PrimitiveTypesBitMap};

use super::{Instruction, SchemaCompiler};

#[derive(Debug, Clone)]
pub(crate) struct TypeSet {
    types: PrimitiveTypesBitMap,
}

impl TypeSet {
    fn new(types: PrimitiveTypesBitMap) -> Self {
        Self { types }
    }

    pub(crate) fn execute(&self, instance: &Value) -> bool {
        match instance {
            Value::Array(_) => self.types.contains_type(PrimitiveType::Array),
            Value::Bool(_) => self.types.contains_type(PrimitiveType::Boolean),
            Value::Null => self.types.contains_type(PrimitiveType::Null),
            Value::Number(num) => {
                self.types.contains_type(PrimitiveType::Number)
                    || (self.types.contains_type(PrimitiveType::Integer) && is_integer(num))
            }
            Value::Object(_) => self.types.contains_type(PrimitiveType::Object),
            Value::String(_) => self.types.contains_type(PrimitiveType::String),
        }
    }
}

fn is_integer(num: &Number) -> bool {
    num.is_u64() || num.is_i64() || num.as_f64().expect("Always valid").fract() == 0.
}

pub(super) fn compile(compiler: &mut SchemaCompiler, obj: &Map<String, Value>) {
    if let Some(Value::String(value)) = obj.get("type") {
        let ty = match value.as_str() {
            "string" => Instruction::TypeString,
            "number" => Instruction::TypeNumber,
            "integer" => Instruction::TypeInteger,
            "boolean" => Instruction::TypeBoolean,
            "array" => Instruction::TypeArray,
            "object" => Instruction::TypeObject,
            "null" => Instruction::TypeNull,
            _ => panic!("Unsupported JSON type: {value}"),
        };

        compiler.emit(ty);
    }
    if let Some(Value::Array(value)) = obj.get("type") {
        let mut types = PrimitiveTypesBitMap::new();
        for item in value {
            match item {
                Value::String(string) => {
                    if let Ok(primitive_type) = PrimitiveType::try_from(string.as_str()) {
                        types |= primitive_type;
                    } else {
                        todo!()
                    }
                }
                _ => {
                    todo!()
                }
            }
        }
        compiler.emit(TypeSet::new(types))
    }
    // TODO: Handle enum, const, etc
}
