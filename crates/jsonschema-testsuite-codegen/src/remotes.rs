use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs::read_to_string,
    path::{Path, MAIN_SEPARATOR},
};

pub(crate) struct RemoteData {
    pub(crate) static_array: TokenStream2,
    pub(crate) resources: Vec<(String, String)>,
}

pub(crate) fn generate(suite_path: &str) -> Result<RemoteData, Box<dyn std::error::Error>> {
    let remotes = Path::new(suite_path).join("remotes");
    if !remotes.exists() || !remotes.is_dir() {
        return Err(format!(
            "Path does not exist or is not a directory: {}. Run `git submodule init && git submodule update`",
            remotes.display()
        )
        .into());
    }

    let mut resources = Vec::new();
    let mut seen_uris = HashSet::new();
    for entry in walkdir::WalkDir::new(&remotes)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path().to_path_buf();
        let relative_path = path.strip_prefix(&remotes).expect("Invalid path");
        let url_path = relative_path
            .to_str()
            .expect("Invalid filename")
            .replace(MAIN_SEPARATOR, "/");
        let uri = format!("http://localhost:1234/{url_path}");
        let contents = read_to_string(path).expect("Failed to read a file");
        seen_uris.insert(uri.clone());
        resources.push((uri, contents.clone()));

        let parsed: Value = serde_json::from_str(&contents).expect("Failed to parse JSON");
        collect_nested_resources(&parsed, &mut resources, &mut seen_uris)?;
    }

    resources.sort_by(|(left_uri, _), (right_uri, _)| left_uri.cmp(right_uri));

    let entries = resources.iter().map(|(uri, contents)| {
        let uri_literal = proc_macro2::Literal::string(uri);
        let contents_literal = proc_macro2::Literal::string(contents);
        quote! { (#uri_literal, #contents_literal) }
    });

    let static_array = quote! {
        static REMOTE_DOCUMENTS: &[(&str, &str)] = &[
            #(#entries),*
        ];
    };

    Ok(RemoteData {
        static_array,
        resources,
    })
}

fn collect_nested_resources(
    value: &Value,
    resources: &mut Vec<(String, String)>,
    seen_uris: &mut HashSet<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    match value {
        Value::Object(map) => {
            let nested_id = map
                .get("$id")
                .and_then(Value::as_str)
                .or_else(|| map.get("id").and_then(Value::as_str));
            if let Some(id) = nested_id {
                if (id.starts_with("http://") || id.starts_with("https://"))
                    && seen_uris.insert(id.to_string())
                {
                    resources.push((id.to_string(), serde_json::to_string(value)?));
                }
            }
            for child in map.values() {
                collect_nested_resources(child, resources, seen_uris)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_nested_resources(child, resources, seen_uris)?;
            }
        }
        _ => {}
    }
    Ok(())
}
