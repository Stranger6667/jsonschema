//! Pre-interned Ruby symbol IDs for avoiding repeated `rb_intern` calls.
//!
//! Ruby's `rb_intern` does a hash table lookup each time it's called with a string.
//! By interning strings once at startup and reusing the resulting IDs, we eliminate
//! repeated lookups in hot paths like error construction.

use std::num::NonZeroUsize;

use magnus::{
    rb_sys::{AsRawId, FromRawId},
    value::{Id, IntoId, StaticSymbol},
    Ruby,
};
use rb_sys::ID;

/// A thread-safe, pre-interned Ruby symbol ID.
///
/// Unlike Magnus's `Id`, this type is `Send + Sync` and can be stored in
/// `static` variables via `LazyLock`. It wraps the raw Ruby `ID` value
/// (which is a process-global, immutable index once interned).
#[derive(Debug, Clone, Copy)]
pub struct StaticId(NonZeroUsize);

// SAFETY: Ruby IDs are process-global, immutable integer indices into the
// symbol table. Once interned, they never change or get collected.
#[allow(unsafe_code)]
unsafe impl Send for StaticId {}
#[allow(unsafe_code)]
unsafe impl Sync for StaticId {}

impl StaticId {
    /// Intern a string and return a `StaticId`.
    ///
    /// # Safety
    ///
    /// Must be called from a Ruby thread.
    #[allow(clippy::cast_possible_truncation, unsafe_code)]
    pub unsafe fn intern(name: &str) -> Self {
        let ruby = unsafe { Ruby::get_unchecked() };
        let id = ruby.intern(name);
        StaticId(unsafe { NonZeroUsize::new_unchecked(id.as_raw() as usize) })
    }

    /// Get the raw Ruby `ID` value.
    #[inline]
    #[allow(clippy::cast_possible_truncation)]
    pub fn as_raw(self) -> ID {
        self.0.get() as ID
    }

    /// Convert to a `StaticSymbol` for use as a hash key.
    #[inline]
    #[allow(unsafe_code)]
    pub fn to_symbol(self) -> StaticSymbol {
        let id: Id = unsafe { Id::from_raw(self.as_raw()) };
        StaticSymbol::from(id)
    }
}

impl IntoId for StaticId {
    #[inline]
    #[allow(unsafe_code)]
    fn into_id_with(self, _handle: &Ruby) -> Id {
        // SAFETY: The raw ID was obtained from a valid `rb_intern` call.
        unsafe { Id::from_raw(self.as_raw()) }
    }
}

/// Define a lazily-interned static Ruby symbol ID.
///
/// The ID is interned on first access (which must happen on a Ruby thread).
///
/// # Example
///
/// ```ignore
/// define_rb_intern!(static ID_MESSAGE: "@message");
/// // Use with funcall:
/// obj.funcall(*ID_ALLOCATE, ())?;
/// // Use with ivar_set:
/// obj.ivar_set(*ID_MESSAGE, value)?;
/// ```
macro_rules! define_rb_intern {
    ($vis:vis static $name:ident : $lit:expr) => {
        #[allow(unsafe_code)]
        $vis static $name: std::sync::LazyLock<$crate::static_id::StaticId> =
            std::sync::LazyLock::new(|| {
                #[allow(unsafe_code)]
                unsafe { $crate::static_id::StaticId::intern($lit) }
            });
    };
}

pub(crate) use define_rb_intern;
