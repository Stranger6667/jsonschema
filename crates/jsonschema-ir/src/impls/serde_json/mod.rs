use serde_json::{Map, Value};
mod value;

use crate::{
    blocks::SubSchema,
    metadata::{Location, LocationId, Metadata, NumberId},
    nodes::Node,
    BlockId, Keyword, Number, ParseError, Schema, ToJsonSchema,
};

impl ToJsonSchema for Value {
    fn to_json_schema(&self) -> Result<Schema, ParseError> {
        match self {
            Value::Bool(value) => Ok(Schema::new_boolean(*value)),
            Value::Object(object) => {
                let mut ctx = ParserContext::new();
                let mut root = ctx.new_block();
                fill_root(&mut ctx, &mut root, object)?;
                let (nested, metadata) = ctx.finish();
                Ok(Schema::new_object(root, nested, metadata))
            }
            _ => Err(ParseError::Invalid),
        }
    }
}

impl From<&serde_json::Number> for NumberId {
    fn from(num: &serde_json::Number) -> Self {
        if let Some(u) = num.as_u64() {
            Number::PositiveInteger(u).into()
        } else if let Some(i) = num.as_i64() {
            Number::NegativeInteger(i).into()
        } else if let Some(f) = num.as_f64() {
            Number::Float(f).into()
        } else {
            panic!("Invalid number encountered in Value")
        }
    }
}

type Object = Map<String, Value>;

fn fill_root(
    ctx: &mut ParserContext,
    root: &mut SubSchema,
    object: &Object,
) -> Result<(), ParseError> {
    if let Some(value) = object.get("minimum") {
        if let Value::Number(value) = value {
            let limit = NumberId::from(value);
            let node = ctx.new_node(Keyword::Minimum { limit });
            root.push(node);
        } else {
            // TODO: Validate that it is a number
        }
    }
    Ok(())
}

struct ParserContext {
    recorded_blocks_count: u32,
    blocks: Vec<SubSchema>,
    location: Location,
    metadata: Metadata,
}

impl ParserContext {
    fn new() -> Self {
        Self {
            recorded_blocks_count: 0,
            blocks: Vec::new(),
            location: Location::new(),
            metadata: Metadata::new(),
        }
    }
    fn finish(self) -> (Vec<SubSchema>, Metadata) {
        (self.blocks, self.metadata)
    }
    fn new_block(&mut self) -> SubSchema {
        let current = self.recorded_blocks_count;
        self.recorded_blocks_count += 1;
        let id = BlockId::new(current);
        SubSchema::new(id)
    }

    fn new_node(&mut self, keyword: Keyword) -> Node {
        let location = self.new_location(keyword.name());
        Node::new(keyword, location)
    }

    fn new_location(&mut self, name: &str) -> LocationId {
        let location = self.location.join(name);
        self.metadata.new_location(location)
    }
}

#[cfg(test)]
mod tests {
    use crate::ToJsonSchema;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(json!(true))]
    #[test_case(json!(false))]
    #[test_case(json!({"minimum": 10}))]
    fn into_json_schema(input: Value) {
        let schema = input.to_json_schema().expect("Failed to parse JSON Schema");
        assert_eq!(
            serde_json::to_string(&input).expect("Failed to serialize JSON"),
            schema.to_string()
        );
    }
}
