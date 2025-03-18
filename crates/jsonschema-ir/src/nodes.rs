use crate::{keywords::Keyword, metadata::location::LocationId};

pub struct Node {
    keyword: Keyword,
    location: LocationId,
}
