use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use std::{
    fs::read_to_string,
    path::{Path, MAIN_SEPARATOR},
};

pub(crate) fn generate(suite_path: &str) -> Result<TokenStream2, Box<dyn std::error::Error>> {
    let remotes = Path::new(suite_path).join("remotes");
    if !remotes.exists() || !remotes.is_dir() {
        return Err(format!(
            "Path does not exist or is not a directory: {}. Run `git submodule init && git submodule update`",
            remotes.display()
        )
        .into());
    }

    let mut resources = Vec::new();
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
        resources.push((uri, contents));
    }

    resources.sort_by(|(left_uri, _), (right_uri, _)| left_uri.cmp(right_uri));

    let entries = resources.iter().map(|(uri, contents)| {
        let uri_literal = proc_macro2::Literal::string(uri);
        let contents_literal = proc_macro2::Literal::string(contents);
        quote! { (#uri_literal, #contents_literal) }
    });

    Ok(quote! {
        static REMOTE_DOCUMENTS: &[(&str, &str)] = &[
            #(#entries),*
        ];
    })
}
