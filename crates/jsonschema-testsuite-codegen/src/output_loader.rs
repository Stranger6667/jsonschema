use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path};

use serde_json::Value;
use testsuite_internal::OutputCase;
use walkdir::WalkDir;

pub(crate) type OutputCaseTree = BTreeMap<String, OutputCaseNode>;

#[derive(Debug)]
pub(crate) enum OutputCaseNode {
    Submodule(OutputCaseTree),
    TestFile(OutputTestFile),
}

#[derive(Debug)]
pub(crate) struct OutputTestFile {
    pub relative_path: String,
    pub cases: Vec<OutputCase>,
}

#[derive(Debug)]
pub(crate) struct OutputSchemaDocuments {
    pub schema: String,
    pub uris: Vec<String>,
}

pub(crate) fn load_cases(
    suite_path: &str,
    version: &str,
) -> Result<OutputCaseTree, Box<dyn std::error::Error>> {
    let content_root = Path::new(suite_path).join(version).join("content");
    if !content_root.exists() {
        return Err(format!("Path does not exist: {}", content_root.display()).into());
    }
    let mut root = OutputCaseTree::new();

    for entry in WalkDir::new(&content_root)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
            let relative_path = path.strip_prefix(&content_root)?;
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            let cases: Vec<OutputCase> = serde_json::from_reader(reader)?;
            insert_into_module_tree(&mut root, relative_path, cases)?;
        }
    }

    Ok(root)
}

pub(crate) fn load_output_schema(
    suite_path: &str,
    version: &str,
) -> Result<OutputSchemaDocuments, Box<dyn std::error::Error>> {
    let schema_path = Path::new(suite_path)
        .join(version)
        .join("output-schema.json");
    let contents = std::fs::read_to_string(&schema_path)?;
    let mut parsed: Value = serde_json::from_str(&contents)?;
    let mut uris = Vec::new();
    if let Some(id) = parsed.get("$id").and_then(Value::as_str) {
        uris.push(id.to_string());
    }
    if let Some(short) = short_reference(version) {
        uris.push(short.to_string());
    }
    normalize_output_schema(&mut parsed, version);
    let schema = serde_json::to_string(&parsed)?;
    if uris.is_empty() {
        return Err(
            format!("Output schema for {version} is missing both $id and short reference").into(),
        );
    }
    Ok(OutputSchemaDocuments { schema, uris })
}

fn insert_into_module_tree(
    tree: &mut OutputCaseTree,
    path: &Path,
    cases: Vec<OutputCase>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut current = tree;

    for component in path.parent().unwrap_or(Path::new("")).components() {
        let key = component.as_os_str().to_string_lossy().into_owned();
        current = current
            .entry(key)
            .or_insert_with(|| OutputCaseNode::Submodule(OutputCaseTree::new()))
            .submodule_mut()?;
    }

    let file_stem = path
        .file_stem()
        .expect("Invalid filename")
        .to_string_lossy()
        .into_owned();
    let relative_path = normalize_path(path);
    current.insert(
        file_stem,
        OutputCaseNode::TestFile(OutputTestFile {
            relative_path,
            cases,
        }),
    );

    Ok(())
}

impl OutputCaseNode {
    fn submodule_mut(&mut self) -> Result<&mut OutputCaseTree, Box<dyn std::error::Error>> {
        match self {
            OutputCaseNode::Submodule(tree) => Ok(tree),
            OutputCaseNode::TestFile(_) => Err("Expected a sub-module, found a test file".into()),
        }
    }
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn normalize_output_schema(value: &mut Value, version: &str) {
    if version == "v1" || version.starts_with("v1-") {
        if let Value::Object(map) = value {
            map.remove("$schema");
        }
    }
}

fn short_reference(version: &str) -> Option<&'static str> {
    if version == "v1" || version.starts_with("v1-") {
        Some("/v1/output/schema")
    } else {
        None
    }
}
