use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};

use testsuite_internal::Case;
use walkdir::WalkDir;

pub(crate) type TestCaseTree = BTreeMap<String, TestCaseNode>;

#[derive(Debug)]
pub(crate) enum TestCaseNode {
    Submodule(TestCaseTree),
    TestFile(Vec<Case>),
}

impl TestCaseNode {
    fn submodule_mut(&mut self) -> Result<&mut TestCaseTree, Box<dyn std::error::Error>> {
        match self {
            TestCaseNode::Submodule(tree) => Ok(tree),
            TestCaseNode::TestFile(_) => Err("Expected a sub-module, found a test file".into()),
        }
    }
}

pub(crate) fn load_suite(
    suite_path: &str,
    draft: &str,
) -> Result<TestCaseTree, Box<dyn std::error::Error>> {
    let full_path = Path::new(suite_path).join("tests").join(draft);
    if !full_path.exists() {
        return Err(format!("Path does not exist: {}", full_path.display()).into());
    }
    let mut root = TestCaseTree::new();

    for entry in WalkDir::new(&full_path).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
            let relative_path = path.strip_prefix(&full_path)?;
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            let cases: Vec<Case> = serde_json::from_reader(reader)?;

            insert_into_module_tree(&mut root, relative_path, cases)?;
        }
    }

    Ok(root)
}

fn insert_into_module_tree(
    tree: &mut TestCaseTree,
    path: &Path,
    cases: Vec<Case>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut current = tree;

    // Navigate through the path components
    for component in path.parent().unwrap_or(Path::new("")).components() {
        let key = component.as_os_str().to_string_lossy().into_owned();
        current = current
            .entry(key)
            .or_insert_with(|| TestCaseNode::Submodule(TestCaseTree::new()))
            .submodule_mut()?;
    }

    // Insert the test file
    let file_name = path
        .file_stem()
        .expect("Invalid filename")
        .to_string_lossy()
        .into_owned();
    current.insert(file_name, TestCaseNode::TestFile(cases));

    Ok(())
}
