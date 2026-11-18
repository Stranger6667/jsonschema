use std::{borrow::Cow, collections::BTreeMap, fs, path::Path};

use testsuite_internal::{Case, SuiteFile};

use crate::files;

pub(crate) type TestCaseTree = BTreeMap<String, TestCaseNode>;

pub(crate) enum TestCaseNode {
    Submodule(TestCaseTree),
    TestFile(DraftFile),
}

impl TestCaseNode {
    fn submodule_mut(&mut self) -> Result<&mut TestCaseTree, Box<dyn std::error::Error>> {
        match self {
            TestCaseNode::Submodule(tree) => Ok(tree),
            TestCaseNode::TestFile(_) => Err("Expected a sub-module, found a test file".into()),
        }
    }
}

/// A suite file's cases that apply to one draft.
pub(crate) struct DraftFile {
    /// Path below `validation/tests`, `/`-separated.
    pub(crate) relative: String,
    pub(crate) cases: Vec<Case>,
}

/// Every non-proposal suite file with at least one case that applies to `draft`.
pub(crate) fn load_files(
    suite_path: &str,
    draft: &str,
) -> Result<Vec<DraftFile>, Box<dyn std::error::Error>> {
    let root = Path::new(suite_path).join("validation").join("tests");
    if !root.is_dir() {
        return Err(format!(
            "Path does not exist or is not a directory: {}. Run `git submodule init && git submodule update`",
            root.display()
        )
        .into());
    }
    let release = release_of(draft);
    let mut output = Vec::new();
    for path in files::json_files(&root)? {
        let relative = path
            .strip_prefix(&root)?
            .to_str()
            .ok_or("Invalid filename")?
            .replace('\\', "/");
        if relative.starts_with("proposals/") {
            continue;
        }
        let contents = fs::read_to_string(&path)?;
        let file: SuiteFile = serde_json::from_str(&sanitize_lone_surrogates(&contents))?;
        let cases: Vec<Case> = file
            .tests
            .into_iter()
            .enumerate()
            .map(|(index, mut case)| {
                case.index = index;
                case
            })
            .filter(|case| {
                case.compatibility
                    .as_deref()
                    .is_none_or(|compatibility| is_compatible(compatibility, release))
            })
            .collect();
        if !cases.is_empty() {
            output.push(DraftFile { relative, cases });
        }
    }
    Ok(output)
}

pub(crate) fn load_suite(
    suite_path: &str,
    draft: &str,
) -> Result<TestCaseTree, Box<dyn std::error::Error>> {
    let mut root = TestCaseTree::new();
    for file in load_files(suite_path, draft)? {
        insert_into_module_tree(&mut root, file)?;
    }
    Ok(root)
}

/// The number the suite's `compatibility` property uses for a draft.
fn release_of(draft: &str) -> u32 {
    match draft {
        "draft4" => 4,
        "draft6" => 6,
        "draft7" => 7,
        "draft2019-09" => 2019,
        _ => 2020,
    }
}

/// Whether a `compatibility` value such as `"6,<=2019"` covers `release`.
pub(crate) fn is_compatible(compatibility: &str, release: u32) -> bool {
    let mut compatible = true;
    for constraint in compatibility.split(',') {
        let (operator, version) = if let Some(version) = constraint.strip_prefix("<=") {
            ("<=", version)
        } else if let Some(version) = constraint.strip_prefix('=') {
            ("=", version)
        } else {
            ("", constraint)
        };
        let version: u32 = version.parse().expect("Compatibility version is a number");
        compatible = match operator {
            "<=" => release <= version,
            "=" => release == version,
            _ => release >= version,
        };
        if release <= version {
            break;
        }
    }
    compatible
}

/// The `format` behavior a case's tags ask for.
pub(crate) fn validate_formats(case: &Case) -> Option<bool> {
    if case.tags.iter().any(|tag| tag == "format-assertion") {
        Some(true)
    } else if case.tags.iter().any(|tag| tag == "format-annotation") {
        Some(false)
    } else {
        None
    }
}

/// Rewrite unpaired surrogate `\uXXXX` escapes to U+FFFD; `serde_json` cannot decode a
/// lone surrogate into a Rust string. Valid pairs are untouched.
pub(crate) fn sanitize_lone_surrogates(input: &str) -> Cow<'_, str> {
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
    file: DraftFile,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut current = tree;
    let mut segments: Vec<&str> = file.relative.split('/').collect();
    let file_name = segments
        .pop()
        .and_then(|name| name.strip_suffix(".json"))
        .expect("Invalid filename")
        .to_owned();

    for segment in segments {
        current = current
            .entry(segment.to_owned())
            .or_insert_with(|| TestCaseNode::Submodule(TestCaseTree::new()))
            .submodule_mut()?;
    }
    current.insert(file_name, TestCaseNode::TestFile(file));

    Ok(())
}
