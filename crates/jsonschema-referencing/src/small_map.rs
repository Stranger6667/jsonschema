use std::mem;

use ahash::AHashMap;

pub(crate) enum SmallMap<K, V, const N: usize = 2> {
    Small(micromap::Map<K, V, N>),
    Large(AHashMap<K, V>),
}

impl<K, V, const N: usize> SmallMap<K, V, N> {
    #[inline]
    pub(crate) fn new() -> Self {
        SmallMap::Small(micromap::Map::new())
    }

    #[inline]
    pub(crate) fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: std::borrow::Borrow<Q> + Eq + std::hash::Hash,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        match self {
            SmallMap::Small(map) => map.get(key),
            SmallMap::Large(map) => map.get(key),
        }
    }

    #[inline]
    pub(crate) fn insert(&mut self, key: K, value: V)
    where
        K: Eq + std::hash::Hash,
    {
        match self {
            SmallMap::Small(map) => {
                // Fits inline (new key with space) or overwrites existing key.
                if map.len() < N || map.get(&key).is_some() {
                    map.insert(key, value);
                    return;
                }
                // Full and key is new — fall through to promotion.
            }
            SmallMap::Large(map) => {
                map.insert(key, value);
                return;
            }
        }
        // Promotion: atomically swap self to Large, drain old Small into it.
        let old = match mem::replace(self, SmallMap::Large(AHashMap::with_capacity(N + 1))) {
            SmallMap::Small(m) => m,
            SmallMap::Large(_) => unreachable!(),
        };
        if let SmallMap::Large(new_map) = self {
            for (k, v) in old {
                new_map.insert(k, v);
            }
            new_map.insert(key, value);
        }
    }

    #[inline]
    pub(crate) fn get_or_insert_default(&mut self, key: K) -> &mut V
    where
        K: Eq + std::hash::Hash,
        V: Default,
    {
        // Determine whether we need to promote before borrowing map contents.
        let needs_promotion = match self {
            SmallMap::Small(map) => map.len() >= N && map.get(&key).is_none(),
            SmallMap::Large(_) => false,
        };
        if needs_promotion {
            // Promotion (same pattern as insert).
            let old = match mem::replace(self, SmallMap::Large(AHashMap::with_capacity(N + 1))) {
                SmallMap::Small(m) => m,
                SmallMap::Large(_) => unreachable!(),
            };
            if let SmallMap::Large(new_map) = self {
                for (k, v) in old {
                    new_map.insert(k, v);
                }
                return new_map.entry(key).or_default();
            }
            unreachable!()
        }
        match self {
            SmallMap::Small(map) => map.entry(key).or_default(),
            SmallMap::Large(map) => map.entry(key).or_default(),
        }
    }

    #[inline]
    pub(crate) fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: std::borrow::Borrow<Q> + Eq + std::hash::Hash,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        self.get(key).is_some()
    }
}

impl<K, V, const N: usize> Default for SmallMap<K, V, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone, V: Clone, const N: usize> Clone for SmallMap<K, V, N> {
    fn clone(&self) -> Self {
        match self {
            SmallMap::Small(map) => SmallMap::Small(map.clone()),
            SmallMap::Large(map) => SmallMap::Large(map.clone()),
        }
    }
}

impl<K: std::fmt::Debug, V: std::fmt::Debug, const N: usize> std::fmt::Debug for SmallMap<K, V, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmallMap::Small(map) => write!(f, "{map:?}"),
            SmallMap::Large(map) => write!(f, "{map:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_is_small() {
        let map: SmallMap<u32, u32> = SmallMap::new();
        assert!(matches!(map, SmallMap::Small(_)));
    }

    #[test]
    fn test_insert_and_get() {
        let mut map: SmallMap<String, u32> = SmallMap::new();
        map.insert("key".to_string(), 42);
        assert_eq!(map.get("key"), Some(&42));
        assert_eq!(map.get("missing"), None);
    }

    #[test]
    fn test_duplicate_key_overwrites() {
        let mut map: SmallMap<String, u32> = SmallMap::new();
        map.insert("key".to_string(), 1);
        map.insert("key".to_string(), 2);
        assert_eq!(map.get("key"), Some(&2));
        // Verify no duplicate was added: inserting the same key again should still return 2
        map.insert("key".to_string(), 2);
        assert_eq!(map.get("key"), Some(&2));
    }

    #[test]
    fn test_multiple_inserts_stay_small() {
        let mut map: SmallMap<u32, u32, 4> = SmallMap::new();
        for i in 0..4 {
            map.insert(i, i * 10);
        }
        assert!(matches!(map, SmallMap::Small(_)));
        for i in 0..4 {
            assert_eq!(map.get(&i), Some(&(i * 10)));
        }
    }

    #[test]
    fn test_promotion_at_n_plus_1() {
        let mut map: SmallMap<u32, u32, 4> = SmallMap::new();
        for i in 0..5 {
            map.insert(i, i * 10);
        }
        assert!(matches!(map, SmallMap::Large(_)));
        for i in 0..5 {
            assert_eq!(map.get(&i), Some(&(i * 10)));
        }
    }

    #[test]
    fn test_contains_key_small() {
        let mut map: SmallMap<String, u32> = SmallMap::new();
        map.insert("a".to_string(), 1);
        assert!(map.contains_key("a"));
        assert!(!map.contains_key("b"));
    }

    #[test]
    fn test_contains_key_large() {
        let mut map: SmallMap<u32, u32, 2> = SmallMap::new();
        map.insert(1, 10);
        map.insert(2, 20);
        map.insert(3, 30); // triggers promotion
        assert!(map.contains_key(&1));
        assert!(map.contains_key(&3));
        assert!(!map.contains_key(&99));
    }

    #[test]
    fn test_get_or_insert_default_miss() {
        let mut map: SmallMap<String, Vec<u32>> = SmallMap::new();
        map.get_or_insert_default("key".to_string()).push(1);
        assert_eq!(map.get("key"), Some(&vec![1u32]));
    }

    #[test]
    fn test_get_or_insert_default_hit() {
        let mut map: SmallMap<String, u32> = SmallMap::new();
        map.insert("key".to_string(), 42);
        let v = map.get_or_insert_default("key".to_string());
        assert_eq!(*v, 42);
        // Verify key was not duplicated: original value still accessible
        assert_eq!(map.get("key"), Some(&42));
    }

    #[test]
    fn test_get_or_insert_default_promotes() {
        let mut map: SmallMap<u32, u32, 2> = SmallMap::new();
        map.insert(1, 10);
        map.insert(2, 20);
        // map is full; inserting new key via get_or_insert_default should promote
        *map.get_or_insert_default(3) = 30;
        assert!(matches!(map, SmallMap::Large(_)));
        assert_eq!(map.get(&3), Some(&30));
        assert_eq!(map.get(&1), Some(&10));
        assert_eq!(map.get(&2), Some(&20));
    }

    #[test]
    fn test_nested_map() {
        let mut outer: SmallMap<u32, SmallMap<String, u32>> = SmallMap::new();
        outer.get_or_insert_default(1).insert("a".to_string(), 10);
        outer.get_or_insert_default(1).insert("b".to_string(), 20);
        outer.get_or_insert_default(2).insert("c".to_string(), 30);
        assert_eq!(outer.get(&1).unwrap().get("a"), Some(&10));
        assert_eq!(outer.get(&1).unwrap().get("b"), Some(&20));
        assert_eq!(outer.get(&2).unwrap().get("c"), Some(&30));
    }

    #[test]
    fn test_clone() {
        let mut map: SmallMap<u32, u32, 4> = SmallMap::new();
        map.insert(1, 10);
        map.insert(2, 20);
        let cloned = map.clone();
        assert_eq!(cloned.get(&1), Some(&10));
        assert_eq!(cloned.get(&2), Some(&20));
    }

    #[test]
    fn test_clone_large() {
        let mut map: SmallMap<u32, u32, 4> = SmallMap::new();
        for i in 0..5 {
            map.insert(i, i * 10);
        }
        let cloned = map.clone();
        for i in 0..5 {
            assert_eq!(cloned.get(&i), Some(&(i * 10)));
        }
    }
}
