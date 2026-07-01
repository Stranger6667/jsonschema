use std::{borrow::Cow, collections::BTreeMap, fs, path::Path};

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
            let contents = fs::read_to_string(path)?;
            let cases: Vec<Case> = serde_json::from_str(&sanitize_lone_surrogates(&contents))?;

            insert_into_module_tree(&mut root, relative_path, cases)?;
        }
    }

    Ok(root)
}

/// Rewrite unpaired surrogate `\uXXXX` escapes to U+FFFD; `serde_json` cannot decode a
/// lone surrogate into a Rust string. Valid pairs are untouched.
fn sanitize_lone_surrogates(input: &str) -> Cow<'_, str> {
    if !input.contains("\\u") {
        return Cow::Borrowed(input);
    }
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if let Some(code) = chars
            .get(i + 1)
            .filter(|c| **c == 'u')
            .and(hex4(&chars, i + 2))
        {
            if (0xD800..=0xDBFF).contains(&code) {
                // High surrogate: valid only if a low surrogate follows.
                let low = (chars.get(i + 6) == Some(&'\\') && chars.get(i + 7) == Some(&'u'))
                    .then(|| hex4(&chars, i + 8))
                    .flatten();
                if low.is_some_and(|low| (0xDC00..=0xDFFF).contains(&low)) {
                    out.extend(&chars[i..i + 12]);
                    i += 12;
                } else {
                    out.push('\u{FFFD}');
                    i += 6;
                }
            } else if (0xDC00..=0xDFFF).contains(&code) {
                out.push('\u{FFFD}');
                i += 6;
            } else {
                out.extend(&chars[i..i + 6]);
                i += 6;
            }
            continue;
        }
        // Other escape: copy verbatim so its char never starts a new escape.
        out.push('\\');
        i += 1;
        if let Some(&c) = chars.get(i) {
            out.push(c);
            i += 1;
        }
    }
    Cow::Owned(out)
}

fn hex4(chars: &[char], start: usize) -> Option<u32> {
    let slice = chars.get(start..start + 4)?;
    slice
        .iter()
        .try_fold(0u32, |acc, c| Some(acc * 16 + c.to_digit(16)?))
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
