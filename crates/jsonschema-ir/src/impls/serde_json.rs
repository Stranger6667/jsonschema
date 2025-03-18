use serde_json::Value;

use crate::{
    schema::{Block, BlockId},
    IntoJsonSchema, ParseError, Schema,
};

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
    fn reserve_block(&mut self) -> BlockId {
        let current = self.blocks;
        self.blocks += 1;
        BlockId(current)
    }
}

fn parse_impl(value: &Value) {
    Block
}

//pub struct Schema {
//    root: Block,
//    nested: Vec<Block>,
//    constants: Constants,
//    paths: Paths,
//}
//
//struct Location(Arc<String>);
//
//pub struct Paths {
//    items: Vec<Location>,
//}
#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(json!(true))]
    fn basic(input: Value) {
        let schema = crate::parse(&input).unwrap();
    }
}
