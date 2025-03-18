use std::sync::Arc;

pub struct LocationId(u32);

#[derive(Debug, Clone)]
struct Location(Arc<String>);

pub struct Locations {
    items: Vec<Location>,
}
