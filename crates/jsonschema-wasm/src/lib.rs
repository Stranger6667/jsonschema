#[cfg(any(target_arch = "wasm32", test))]
mod errors;
#[cfg(any(target_arch = "wasm32", test))]
mod options;

#[cfg(target_arch = "wasm32")]
mod wasm_exports;
#[cfg(target_arch = "wasm32")]
pub use wasm_exports::{bundle, dereference, drafts, meta_validate, validate, version};
