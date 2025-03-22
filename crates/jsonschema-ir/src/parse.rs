use crate::{error::ParseError, schema::Schema};

pub trait ToJsonSchema {
    fn to_json_schema(&self) -> Result<Schema, ParseError>;
}
