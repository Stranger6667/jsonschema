use crate::{keywords::Keyword, metadata::LocationId};

pub struct Node {
    keyword: Keyword,
    location: LocationId,
}

impl Node {
    pub(crate) fn new(keyword: Keyword, location: LocationId) -> Self {
        Self { keyword, location }
    }
}
