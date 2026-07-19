use std::path::{Path, PathBuf};

use walkdir::{DirEntry, WalkDir};

/// Every `*.json` file under `dir`, in a stable order.
pub(crate) fn json_files(dir: &Path) -> Result<impl Iterator<Item = PathBuf>, String> {
    if !dir.exists() {
        return Err(format!("Path does not exist: {}", dir.display()));
    }
    Ok(WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .map(DirEntry::into_path)
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "json")))
}
