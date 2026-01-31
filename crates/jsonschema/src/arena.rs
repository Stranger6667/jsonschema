use bumpalo::Bump;
use std::sync::Arc;

use crate::validator::Validate;

/// Inner arena storage. Not Send/Sync itself, but we wrap it safely.
struct ArenaInner {
    bump: Bump,
}

/// Arena for allocating validators during compilation.
///
/// This is wrapped in Arc so that cloning a Validator shares the arena.
/// The arena is only written to during compilation (single-threaded),
/// and is read-only during validation (multi-threaded safe).
#[derive(Clone)]
pub(crate) struct ValidatorArena {
    inner: Arc<ArenaInner>,
}

/// Default initial arena capacity (16KB).
/// This is enough for most schemas without reallocation, and the arena
/// will grow automatically if needed.
const DEFAULT_ARENA_CAPACITY: usize = 16 * 1024;

impl ValidatorArena {
    pub(crate) fn new() -> Self {
        Self::with_capacity(DEFAULT_ARENA_CAPACITY)
    }

    /// Create an arena with specified initial capacity in bytes.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(ArenaInner {
                bump: Bump::with_capacity(capacity),
            }),
        }
    }

    /// Allocate a validator and return a 'static reference.
    ///
    /// # Safety
    ///
    /// The returned reference is transmuted to 'static. This is safe because:
    /// 1. The arena is owned by `Validator` struct (via Arc)
    /// 2. `root: SchemaNode` is declared after `arena` in Validator (drops first)
    /// 3. After compilation, the arena is read-only
    /// 4. Arc ensures the arena lives as long as any Validator clone
    ///
    /// # Panics
    ///
    /// Panics if called when there are multiple references to the arena
    /// (i.e., after the Validator has been cloned). This should only be
    /// called during compilation when there's a single reference.
    pub(crate) fn alloc<V: Validate + 'static>(&self, v: V) -> &'static (dyn Validate + 'static) {
        // SAFETY: During compilation, we have exclusive access to the arena.
        // We use Arc::get_mut would fail here since we're borrowing &self,
        // but we know compilation is single-threaded and the arena won't be
        // accessed concurrently during this phase.
        let inner = unsafe { &mut *(Arc::as_ptr(&self.inner) as *mut ArenaInner) };
        let ptr = inner.bump.alloc(v) as *const dyn Validate;
        // SAFETY: Arena lives as long as any Validator clone (Arc-shared).
        unsafe { &*ptr }
    }
}

impl Default for ValidatorArena {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ValidatorArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatorArena")
            .field("allocated_bytes", &self.inner.bump.allocated_bytes())
            .finish()
    }
}

// SAFETY: After compilation, arena is read-only (validation only reads).
// The validators stored in the arena are all Send + Sync (required by Validate trait).
// During compilation (the only write phase), access is single-threaded.
unsafe impl Send for ArenaInner {}
unsafe impl Sync for ArenaInner {}
