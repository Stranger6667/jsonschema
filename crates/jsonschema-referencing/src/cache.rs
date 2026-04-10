use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use fluent_uri::Uri;
use hashbrown::{Equivalent, HashMap};
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use crate::{uri, Error};

type CacheMap = HashMap<(String, String), Arc<Uri<String>>>;

struct StrPair<'a>(&'a str, &'a str);

impl Hash for StrPair<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
        self.1.hash(state);
    }
}

impl Equivalent<(String, String)> for StrPair<'_> {
    fn equivalent(&self, (a, b): &(String, String)) -> bool {
        self.0 == a && self.1 == b
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UriCache {
    cache: CacheMap,
}

impl UriCache {
    pub(crate) fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: HashMap::with_capacity(capacity),
        }
    }

    pub(crate) fn resolve_against(
        &mut self,
        base: &Uri<&str>,
        uri: impl AsRef<str>,
    ) -> Result<Arc<Uri<String>>, Error> {
        let base_str = base.as_str();
        let reference = uri.as_ref();
        if let Some(cached) = self.cache.get(&StrPair(base_str, reference)) {
            return Ok(Arc::clone(cached));
        }
        let resolved = Arc::new(uri::resolve_against(base, reference)?);
        self.cache.insert(
            (base_str.to_owned(), reference.to_owned()),
            Arc::clone(&resolved),
        );
        Ok(resolved)
    }

    pub(crate) fn into_shared(self) -> SharedUriCache {
        SharedUriCache {
            cache: RwLock::new(self.cache),
        }
    }
}

/// A dedicated type for URI resolution caching.
#[derive(Debug)]
pub(crate) struct SharedUriCache {
    cache: RwLock<CacheMap>,
}

impl Clone for SharedUriCache {
    fn clone(&self) -> Self {
        Self {
            cache: RwLock::new(
                self.cache
                    .read()
                    .iter()
                    .map(|(key, value)| (key.clone(), Arc::clone(value)))
                    .collect(),
            ),
        }
    }
}

impl SharedUriCache {
    pub(crate) fn resolve_against(
        &self,
        base: &Uri<&str>,
        uri: impl AsRef<str>,
    ) -> Result<Arc<Uri<String>>, Error> {
        let base_str = base.as_str();
        let reference = uri.as_ref();
        let lookup = StrPair(base_str, reference);

        if let Some(cached) = self.cache.read().get(&lookup) {
            return Ok(Arc::clone(cached));
        }

        let cache = self.cache.upgradable_read();
        if let Some(cached) = cache.get(&lookup) {
            return Ok(Arc::clone(cached));
        }

        let resolved = Arc::new(uri::resolve_against(base, reference)?);
        let mut cache = RwLockUpgradableReadGuard::upgrade(cache);
        cache.insert(
            (base_str.to_owned(), reference.to_owned()),
            Arc::clone(&resolved),
        );
        Ok(resolved)
    }
}
