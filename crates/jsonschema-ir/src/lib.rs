mod blocks;
mod error;
mod impls;
mod keywords;
mod metadata;
mod nodes;
mod parse;
mod schema;
mod value;

pub use blocks::BlockId;
pub use error::ParseError;
pub use keywords::Keyword;
pub use parse::ToJsonSchema;
pub use schema::Schema;
pub use value::{JsonValue, Number};

pub fn parse<I>(input: I) -> Result<Schema, ParseError>
where
    I: ToJsonSchema,
{
    input.to_json_schema()
}
