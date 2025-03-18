mod blocks;
mod error;
mod impls;
mod keywords;
mod metadata;
mod nodes;
mod parse;
mod schema;

pub use blocks::BlockId;
pub use error::ParseError;
pub use keywords::Keyword;
pub use parse::IntoJsonSchema;
pub use schema::Schema;

pub fn parse<I>(input: I) -> Result<Schema, ParseError>
where
    I: IntoJsonSchema,
{
    input.parse()
}
