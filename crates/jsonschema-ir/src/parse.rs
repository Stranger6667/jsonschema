use crate::{error::ParseError, schema::Schema};

pub trait IntoJsonSchema {
    fn parse(&self) -> Result<Schema, ParseError>;
}
