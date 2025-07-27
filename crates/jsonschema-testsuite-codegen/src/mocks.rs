use std::{
    fs::read_to_string,
    path::{Path, MAIN_SEPARATOR},
};

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

pub(crate) fn generate(suite_path: &str) -> Result<TokenStream2, Box<dyn std::error::Error>> {
    let remotes = Path::new(suite_path).join("remotes");

    let mut mock = quote! {
        let mut server = mockito::Server::new_with_opts(mockito::ServerOpts {
            port: 1234,
            ..Default::default()
        });
    };

    if remotes.exists() && remotes.is_dir() {
        for entry in walkdir::WalkDir::new(&remotes)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let relative_path = path.strip_prefix(&remotes).expect("Invalid path");
            let url_path = relative_path
                .to_str()
                .expect("Invalid filename")
                .replace(MAIN_SEPARATOR, "/");
            let content = read_to_string(path).expect("Failed to read a file");

            mock = quote! {
                #mock
                server.mock("GET", format!("/{}", #url_path).as_str())
                    .with_status(200)
                    .with_header("content-type", "application/json")
                    .with_body(#content)
                    .create();
            }
        }
    } else {
        return Err(format!(
            "Path does not exist or is not a directory: {}. Run `git submodule init && git submodule update`",
            remotes.display()
        )
        .into());
    }

    Ok(quote! {
        #mock
        server
    })
}
