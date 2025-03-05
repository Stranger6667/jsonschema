use crate::paths::Location;
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct NodeMetadata {
    pub(crate) location: Location,
    pub(crate) keyword_value: Value,
}
