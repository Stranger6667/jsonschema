use crate::paths::{Location, LocationSegment};

/// Tracks the location within the input schema.
pub(crate) struct LocationContext {
    stack: Vec<Location>,
    top: Location,
}

impl LocationContext {
    pub(crate) fn new() -> Self {
        Self {
            stack: Vec::new(),
            top: Location::new(),
        }
    }
    /// Push a new location.
    pub(crate) fn push(&mut self, key: &str) {
        let mut new = self.top.join(key);
        std::mem::swap(&mut self.top, &mut new);
        self.stack.push(new);
    }
    /// Remove the last location.
    pub(crate) fn pop(&mut self) {
        let mut top = self.stack.pop().expect("Empty stack");
        std::mem::swap(&mut self.top, &mut top);
    }
    /// Create a new `Location` for the given segment
    pub(crate) fn join<'a>(&mut self, segment: impl Into<LocationSegment<'a>>) -> Location {
        self.top.join(segment)
    }
}
