use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Expr, ExprArray, ExprLit, Lit, Meta, Token,
};

/// Extract a string-literal value from a `name = "..."` attribute.
fn str_value(name_value: syn::MetaNameValue, what: &str) -> syn::Result<String> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Str(lit), ..
    }) = name_value.value
    {
        Ok(lit.value())
    } else {
        Err(syn::Error::new_spanned(name_value.value, what))
    }
}

#[must_use]
pub fn sanitize_name(s: String) -> String {
    match s.as_str() {
        "const" | "enum" | "ref" | "type" => format!("r#{s}"),
        _ => s,
    }
}
/// Configuration for the `suite` attribute.
pub struct SuiteConfig {
    pub path: String,
    pub drafts: Vec<String>,
    pub xfail: Vec<String>,
}

impl Parse for SuiteConfig {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut path = None;
        let mut drafts = Vec::new();
        let mut xfail = Vec::new();

        for meta in Punctuated::<Meta, Token![,]>::parse_terminated(input)? {
            match meta {
                Meta::NameValue(name_value) if name_value.path.is_ident("path") => {
                    path = Some(str_value(
                        name_value,
                        "Test suite path should be a string literal",
                    )?);
                }
                Meta::NameValue(name_value) if name_value.path.is_ident("drafts") => {
                    if let Expr::Array(ExprArray { elems, .. }) = name_value.value {
                        for elem in elems {
                            if let Expr::Lit(ExprLit {
                                lit: Lit::Str(lit), ..
                            }) = elem
                            {
                                drafts.push(lit.value());
                            } else {
                                return Err(syn::Error::new_spanned(
                                    elem,
                                    "Drafts name should be a string literal",
                                ));
                            }
                        }
                    } else {
                        return Err(syn::Error::new_spanned(
                            name_value.value,
                            "Drafts should be an array of string literals",
                        ));
                    }
                }
                Meta::NameValue(name_value) if name_value.path.is_ident("xfail") => {
                    if let Expr::Array(ExprArray { elems, .. }) = name_value.value {
                        for elem in elems {
                            if let Expr::Lit(ExprLit {
                                lit: Lit::Str(lit), ..
                            }) = elem
                            {
                                xfail.push(lit.value());
                            } else {
                                return Err(syn::Error::new_spanned(
                                    elem,
                                    "XFail item should be a string literal",
                                ));
                            }
                        }
                    } else {
                        return Err(syn::Error::new_spanned(
                            name_value.value,
                            "XFail should be an array of string literals",
                        ));
                    }
                }
                _ => return Err(syn::Error::new_spanned(meta, "Unexpected attribute")),
            }
        }
        let path = path.ok_or_else(|| {
            syn::Error::new(input.span(), "Missing path to JSON Schema test suite")
        })?;
        if drafts.is_empty() {
            return Err(syn::Error::new(input.span(), "Drafts are missing"));
        }

        Ok(SuiteConfig {
            path,
            drafts,
            xfail,
        })
    }
}

/// Configuration for the `canonical_suite` attribute — a single `path` to a
/// directory of `*.json` case files.
pub struct CanonicalSuiteConfig {
    pub path: String,
}

impl Parse for CanonicalSuiteConfig {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut path = None;
        for meta in Punctuated::<Meta, Token![,]>::parse_terminated(input)? {
            match meta {
                Meta::NameValue(name_value) if name_value.path.is_ident("path") => {
                    path = Some(str_value(
                        name_value,
                        "Canonical suite path should be a string literal",
                    )?);
                }
                _ => return Err(syn::Error::new_spanned(meta, "Unexpected attribute")),
            }
        }
        let path =
            path.ok_or_else(|| syn::Error::new(input.span(), "Missing path to canonical suite"))?;
        Ok(CanonicalSuiteConfig { path })
    }
}
