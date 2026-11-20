use std::sync::Arc;

/// An immutable singly-linked list.
pub struct List<T> {
    head: Option<Arc<Node<T>>>,
}

impl<T: Clone> Clone for List<T> {
    fn clone(&self) -> Self {
        List {
            head: self.head.clone(),
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for List<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: PartialEq> PartialEq for List<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.head, &other.head) {
            (Some(left), Some(right)) if Arc::ptr_eq(left, right) => return true,
            _ => {}
        }
        self.iter().eq(other.iter())
    }
}

impl<T: Eq> Eq for List<T> {}

impl<T: std::hash::Hash> std::hash::Hash for List<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for item in self {
            item.hash(state);
        }
    }
}

impl<T> Drop for List<T> {
    fn drop(&mut self) {
        let mut current = self.head.take();
        while let Some(node) = current {
            if let Ok(mut node) = Arc::try_unwrap(node) {
                current = node.next.take();
            } else {
                break;
            }
        }
    }
}

impl<T> List<T> {
    pub(crate) fn new() -> Self {
        Self { head: None }
    }
    /// Returns true if the list contains no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }
    /// Creates a new list with the given value at the front, sharing the rest of the nodes.
    #[must_use]
    pub fn push_front(&self, value: Arc<T>) -> Self {
        List {
            head: Some(Arc::new(Node {
                value,
                next: self.head.clone(),
            })),
        }
    }
    /// Returns an iterator over references to the list elements.
    #[must_use]
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            current: self.head.as_ref(),
        }
    }
    /// An iterator over the shared pointers to the list elements.
    pub(crate) fn iter_shared(&self) -> impl Iterator<Item = &Arc<T>> {
        std::iter::successors(self.head.as_ref(), |node| node.next.as_ref()).map(|node| &node.value)
    }
    /// The elements in front of `tail`, when this list continues with the elements of `tail`.
    pub(crate) fn prefix_before(&self, tail: &List<T>) -> Option<std::iter::Take<Iter<'_, T>>>
    where
        T: PartialEq,
    {
        // Lists sharing their nodes with `tail` meet at its first node.
        let mut length = 0;
        let mut current = self.head.as_ref();
        loop {
            match (current, tail.head.as_ref()) {
                (Some(node), Some(end)) if Arc::ptr_eq(node, end) => {
                    return Some(self.iter().take(length))
                }
                (None, None) => return Some(self.iter().take(length)),
                (None, Some(_)) => break,
                (Some(node), _) => {
                    length += 1;
                    current = node.next.as_ref();
                }
            }
        }
        let length = length.checked_sub(tail.iter().count())?;
        self.iter()
            .skip(length)
            .eq(tail.iter())
            .then(|| self.iter().take(length))
    }
}

#[derive(Debug)]
pub(crate) struct Node<T> {
    value: Arc<T>,
    next: Option<Arc<Node<T>>>,
}

/// Iterator over references to elements in a `List`.
#[derive(Debug)]
pub struct Iter<'a, T> {
    current: Option<&'a Arc<Node<T>>>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.current.map(|current| {
            let value = &current.value;
            self.current = current.next.as_ref();
            &**value
        })
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type IntoIter = Iter<'a, T>;
    type Item = &'a T;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
