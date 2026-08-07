use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::Arc,
};

use super::Schema;

type Entry = (Arc<str>, Schema);

/// A key-to-schema map kept sorted by key. Nearly every object leaf names one key or none, so those
/// two shapes live in the value itself and never reach the allocator.
#[derive(Debug, Clone, Default)]
pub(crate) enum PropertyMap {
    #[default]
    Empty,
    One([Entry; 1]),
    Many(Vec<Entry>),
}

impl PropertyMap {
    #[must_use]
    pub(crate) fn as_slice(&self) -> &[Entry] {
        match self {
            Self::Empty => &[],
            Self::One(entry) => entry,
            Self::Many(entries) => entries,
        }
    }

    fn as_mut_slice(&mut self) -> &mut [Entry] {
        match self {
            Self::Empty => &mut [],
            Self::One(entry) => entry,
            Self::Many(entries) => entries,
        }
    }

    /// Rebuilds the shape a sorted entry list calls for.
    fn from_sorted(mut entries: Vec<Entry>) -> Self {
        debug_assert!(
            entries.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "property entries left unsorted or duplicated"
        );
        match entries.len() {
            0 => Self::Empty,
            1 => Self::One([entries.pop().expect("one entry")]),
            _ => Self::Many(entries),
        }
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.as_slice().len()
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    #[must_use]
    pub(crate) fn get(&self, key: &str) -> Option<&Schema> {
        let entries = self.as_slice();
        entries
            .binary_search_by(|entry| (*entry.0).cmp(key))
            .ok()
            .map(|index| &entries[index].1)
    }

    #[must_use]
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &Arc<str>> {
        self.as_slice().iter().map(|entry| &entry.0)
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &Schema> {
        self.as_slice().iter().map(|entry| &entry.1)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &Schema)> {
        self.as_slice().iter().map(|entry| (&entry.0, &entry.1))
    }

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut Schema> {
        self.as_mut_slice().iter_mut().map(|entry| &mut entry.1)
    }

    /// Adds the schema `missing` yields when the key is absent, and hands back the schema at the key.
    pub(crate) fn or_insert_with(
        &mut self,
        key: Arc<str>,
        missing: impl FnOnce() -> Schema,
    ) -> &mut Schema {
        let index = match self
            .as_slice()
            .binary_search_by(|entry| (*entry.0).cmp(&key))
        {
            Ok(index) => index,
            Err(index) => {
                self.insert_absent(index, key, missing());
                index
            }
        };
        &mut self.as_mut_slice()[index].1
    }

    /// Places an entry the map does not hold at the sorted position `index`.
    fn insert_absent(&mut self, index: usize, key: Arc<str>, schema: Schema) {
        match std::mem::take(self) {
            Self::Empty => *self = Self::One([(key, schema)]),
            Self::One([held]) => {
                let mut entries = Vec::with_capacity(2);
                entries.push(held);
                entries.insert(index, (key, schema));
                *self = Self::Many(entries);
            }
            Self::Many(mut entries) => {
                entries.insert(index, (key, schema));
                *self = Self::Many(entries);
            }
        }
    }

    pub(crate) fn insert(&mut self, key: Arc<str>, schema: Schema) -> Option<Schema> {
        match self
            .as_mut_slice()
            .binary_search_by(|entry| (*entry.0).cmp(&key))
        {
            Ok(index) => Some(std::mem::replace(&mut self.as_mut_slice()[index].1, schema)),
            Err(index) => {
                self.insert_absent(index, key, schema);
                None
            }
        }
    }

    pub(crate) fn remove(&mut self, key: &str) -> Option<Schema> {
        let index = self
            .as_slice()
            .binary_search_by(|entry| (*entry.0).cmp(key))
            .ok()?;
        match std::mem::take(self) {
            Self::Empty => unreachable!("the search found the key, so the map holds it"),
            Self::One([(_, schema)]) => Some(schema),
            Self::Many(mut entries) => {
                let (_, schema) = entries.remove(index);
                *self = Self::from_sorted(entries);
                Some(schema)
            }
        }
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&Arc<str>, &mut Schema) -> bool) {
        match self {
            Self::Empty => {}
            Self::One([(key, schema)]) => {
                if !keep(key, schema) {
                    *self = Self::Empty;
                }
            }
            Self::Many(entries) => {
                entries.retain_mut(|(key, schema)| keep(key, schema));
                let entries = std::mem::take(entries);
                *self = Self::from_sorted(entries);
            }
        }
    }
}

impl PartialEq for PropertyMap {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for PropertyMap {}

impl PartialOrd for PropertyMap {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PropertyMap {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl Hash for PropertyMap {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl FromIterator<Entry> for PropertyMap {
    fn from_iter<T: IntoIterator<Item = Entry>>(iter: T) -> Self {
        let mut entries: Vec<Entry> = iter.into_iter().collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries.dedup_by(|left, right| left.0 == right.0);
        Self::from_sorted(entries)
    }
}

impl<'a> IntoIterator for &'a PropertyMap {
    type Item = (&'a Arc<str>, &'a Schema);
    type IntoIter = Box<dyn Iterator<Item = (&'a Arc<str>, &'a Schema)> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}
