use std::{fs::File, io::BufReader, path::Path};

use referencing_internal::Case;
use walkdir::WalkDir;

#[derive(Debug)]
pub(crate) struct TestCase {
    pub(crate) name: String,
    pub(crate) case: Case,
}

pub(crate) fn load_suite(
    suite_path: &str,
    draft: &str,
) -> Result<Vec<TestCase>, Box<dyn std::error::Error>> {
    let full_path = Path::new(suite_path).join("tests").join(draft);
    if !full_path.exists() {
        return Err(format!("Path does not exist: {}", full_path.display()).into());
    }
    let mut suite = Vec::with_capacity(60);

    for entry in WalkDir::new(&full_path).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            let case: Case = serde_json::from_reader(reader)?;
            suite.push(TestCase {
                name: path
                    .file_stem()
                    .expect("Invalid filename")
                    .to_string_lossy()
                    .to_string(),
                case,
            });
        }
    }
    Ok(suite)
}
