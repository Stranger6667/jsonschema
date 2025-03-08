use serde_json::{Map, Number, Value};

use crate::{
    keywords::helpers::equal,
    primitive_type::{PrimitiveType, PrimitiveTypesBitMap},
};

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

#[derive(Debug)]
pub struct Enum {
    values: Vec<Value>,
}

impl Enum {
    pub fn new(values: Vec<Value>) -> Enum {
        Enum { values }
    }

    pub fn execute(&self, value: &Value) -> bool {
        // Use the provided equal function to compare values
        self.values.iter().any(|v| equal(value, v))
    }
}

#[derive(Debug)]
pub struct EnumSingle {
    value: Value,
}
impl EnumSingle {
    fn new(value: Value) -> EnumSingle {
        EnumSingle { value }
    }

    pub(crate) fn execute(&self, value: &Value) -> bool {
        equal(value, &self.value)
    }
}

pub(super) fn compile(
    compiler: &mut SchemaCompiler,
    obj: &Map<String, Value>,
    jumps: &mut Vec<usize>,
) {
    if let Some(ty) = obj.get("type") {
        match ty {
            Value::String(ty) => {
                let ty = match ty.as_str() {
                    "string" => Instruction::TypeString,
                    "number" => Instruction::TypeNumber,
                    "integer" => Instruction::TypeInteger,
                    "boolean" => Instruction::TypeBoolean,
                    "array" => Instruction::TypeArray,
                    "object" => Instruction::TypeObject,
                    "null" => Instruction::TypeNull,
                    _ => panic!("Unsupported JSON type: {ty}"),
                };
                compiler.emit(ty);
                jumps.push(compiler.emit_jump_if_invalid());
            }
            Value::Array(types) => {
                let mut set = PrimitiveTypesBitMap::new();
                for item in types {
                    match item {
                        Value::String(string) => {
                            if let Ok(ty) = PrimitiveType::try_from(string.as_str()) {
                                set |= ty;
                            } else {
                                todo!()
                            }
                        }
                        _ => {
                            todo!()
                        }
                    }
                }
                compiler.emit(TypeSet::new(set));
                jumps.push(compiler.emit_jump_if_invalid());
            }
            _ => todo!(),
        }
    }
    if let Some(Value::Array(values)) = obj.get("enum") {
        match values.as_slice() {
            [value] => {
                compiler.emit(EnumSingle::new(value.clone()));
            }
            _ => {
                compiler.emit(Enum::new(values.clone()));
            }
        }
        jumps.push(compiler.emit_jump_if_invalid());
    }
    // TODO: Handle const, etc
}
