use serde_json::Value;
mod value;

use crate::{blocks::Block, value::Number, BlockId, IntoJsonSchema, JsonValue, ParseError, Schema};

impl<'a> IntoJsonSchema for &'a Value {
    fn parse(&self) -> Result<Schema, ParseError> {
        match self {
            Value::Bool(true) => todo!(),
            Value::Bool(false) => todo!(),
            Value::Object(map) => todo!(),
            _ => Err(ParseError::Invalid),
        }
    }
}

struct ParserContext {
    blocks: u32,
}

impl ParserContext {
    fn new() -> Self {
        Self { blocks: 0 }
    }
    fn new_block(&mut self) -> Block {
        let current = self.blocks;
        self.blocks += 1;
        let id = BlockId::new(current);
        Block::new(id)
    }
}

fn parse_impl(value: &Value) {}
