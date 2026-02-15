use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::context::{CompileContext, PatternEngineConfig};

use super::stack_emit::{
    pop_dynamic_item_eval_n, pop_dynamic_key_eval_n, pop_dynamic_validate_n,
    pop_recursive_item_eval, pop_recursive_key_eval, pop_recursive_validate,
    push_dynamic_item_eval, push_dynamic_key_eval, push_dynamic_validate, push_recursive_item_eval,
    push_recursive_key_eval, push_recursive_validate,
};

pub(super) fn emit_root_module(
    ctx: &CompileContext<'_>,
    runtime_crate_alias: Option<&TokenStream>,
    recompile_trigger: &TokenStream,
    name: &Ident,
    impl_mod_name: &Ident,
    validation_expr: &TokenStream,
    recursive_stack_needed: bool,
    dynamic_stack_needed: bool,
    root_recursive_anchor: bool,
    root_key_eval_ident: Option<&Ident>,
    root_item_eval_ident: Option<&Ident>,
    root_dynamic_bindings: &[(String, String, String, String)],
) -> TokenStream {
    let emit_symbols = ctx.config.backend.emit_symbols();
    let value_ty = emit_symbols.value_ty();
    let map_ty = emit_symbols.map_ty();
    let value_slice_ty = emit_symbols.value_slice_ty();

    let regex_helpers: Vec<TokenStream> = ctx
        .regex_helpers
        .iter()
        .map(|(name, pattern)| {
            let helper_ident = format_ident!("{}", name);
            match ctx.config.pattern_options {
                PatternEngineConfig::FancyRegex {
                    backtrack_limit,
                    size_limit,
                    dfa_size_limit,
                } => {
                    let set_backtrack_limit =
                        backtrack_limit.map(|limit| quote! { builder.backtrack_limit(#limit); });
                    let set_size_limit = size_limit
                        .map(|limit| quote! { builder.delegate_size_limit(#limit); });
                    let set_dfa_size_limit = dfa_size_limit
                        .map(|limit| quote! { builder.delegate_dfa_size_limit(#limit); });
                    quote! {
                        #[inline]
                        fn #helper_ident(subject: &str) -> bool {
                            static REGEX: std::sync::LazyLock<Option<jsonschema::__private::fancy_regex::Regex>> =
                                std::sync::LazyLock::new(|| {
                                    let mut builder = jsonschema::__private::fancy_regex::RegexBuilder::new(#pattern);
                                    #set_backtrack_limit
                                    #set_size_limit
                                    #set_dfa_size_limit
                                    builder.build().ok()
                                });
                            REGEX
                                .as_ref()
                                .is_some_and(|re| re.is_match(subject).unwrap_or(false))
                        }
                    }
                }
                PatternEngineConfig::Regex {
                    size_limit,
                    dfa_size_limit,
                } => {
                    let set_size_limit =
                        size_limit.map(|limit| quote! { builder.size_limit(#limit); });
                    let set_dfa_size_limit =
                        dfa_size_limit.map(|limit| quote! { builder.dfa_size_limit(#limit); });
                    quote! {
                        #[inline]
                        fn #helper_ident(subject: &str) -> bool {
                            static REGEX: std::sync::LazyLock<Option<jsonschema::__private::regex::Regex>> =
                                std::sync::LazyLock::new(|| {
                                    let mut builder = jsonschema::__private::regex::RegexBuilder::new(#pattern);
                                    #set_size_limit
                                    #set_dfa_size_limit
                                    builder.build().ok()
                                });
                            REGEX.as_ref().is_some_and(|re| re.is_match(subject))
                        }
                    }
                }
            }
        })
        .collect();

    let is_valid_helpers: Vec<TokenStream> = ctx
        .is_valid_bodies
        .iter()
        .map(|(fname, body)| {
            let func_ident = format_ident!("{}", fname);
            quote! {
                #[inline]
                fn #func_ident(instance: &#value_ty) -> bool { #body }
            }
        })
        .collect();

    let eval_helpers: Vec<TokenStream> = ctx
        .eval_bodies
        .iter()
        .map(|(fname, body)| {
            let func_ident = format_ident!("{}", fname);
            quote! {
                #[inline]
                fn #func_ident(
                    instance: &#value_ty,
                    obj: &#map_ty,
                    key_str: &str
                ) -> bool { #body }
            }
        })
        .collect();

    let item_eval_helpers: Vec<TokenStream> = ctx
        .item_eval_bodies
        .iter()
        .map(|(fname, body)| {
            let func_ident = format_ident!("{}", fname);
            quote! {
                #[inline]
                fn #func_ident(
                    instance: &#value_ty,
                    arr: &#value_slice_ty,
                    idx: usize,
                    item: &#value_ty
                ) -> bool { #body }
            }
        })
        .collect();

    let recursive_stack_defs = if recursive_stack_needed {
        quote! {
            static __JSONSCHEMA_RECURSIVE_STACK: std::cell::RefCell<Vec<(fn(&#value_ty) -> bool, bool)>> =
                std::cell::RefCell::new(Vec::new());
            static __JSONSCHEMA_RECURSIVE_KEY_EVAL_STACK: std::cell::RefCell<
                Vec<(fn(&#value_ty, &#map_ty, &str) -> bool, bool)>
            > = std::cell::RefCell::new(Vec::new());
            static __JSONSCHEMA_RECURSIVE_ITEM_EVAL_STACK: std::cell::RefCell<
                Vec<(fn(&#value_ty, &#value_slice_ty, usize, &#value_ty) -> bool, bool)>
            > = std::cell::RefCell::new(Vec::new());
        }
    } else {
        quote! {}
    };
    let dynamic_stack_defs = if dynamic_stack_needed {
        quote! {
            static __JSONSCHEMA_DYNAMIC_STACK: std::cell::RefCell<
                Vec<(&'static str, fn(&#value_ty) -> bool)>
            > = std::cell::RefCell::new(Vec::new());
            static __JSONSCHEMA_DYNAMIC_KEY_EVAL_STACK: std::cell::RefCell<
                Vec<(
                    &'static str,
                    fn(&#value_ty, &#map_ty, &str) -> bool,
                )>
            > = std::cell::RefCell::new(Vec::new());
            static __JSONSCHEMA_DYNAMIC_ITEM_EVAL_STACK: std::cell::RefCell<
                Vec<(
                    &'static str,
                    fn(&#value_ty, &#value_slice_ty, usize, &#value_ty) -> bool,
                )>
            > = std::cell::RefCell::new(Vec::new());
        }
    } else {
        quote! {}
    };
    let recursive_stack = if recursive_stack_needed || dynamic_stack_needed {
        quote! {
            std::thread_local! {
                #recursive_stack_defs
                #dynamic_stack_defs
            }
        }
    } else {
        quote! {}
    };

    let root_key_eval_ident = root_key_eval_ident
        .cloned()
        .unwrap_or_else(|| format_ident!("__unused_root_key_eval"));
    let root_item_eval_ident = root_item_eval_ident
        .cloned()
        .unwrap_or_else(|| format_ident!("__unused_root_item_eval"));
    let is_valid_ident = format_ident!("is_valid");
    let recursive_push = if recursive_stack_needed {
        let push_validate = push_recursive_validate(&is_valid_ident, root_recursive_anchor);
        let push_key = push_recursive_key_eval(&root_key_eval_ident, root_recursive_anchor);
        let push_item = push_recursive_item_eval(&root_item_eval_ident, root_recursive_anchor);
        quote! {
            #push_validate
            #push_key
            #push_item
        }
    } else {
        quote! {}
    };
    let recursive_pop = if recursive_stack_needed {
        let pop_item = pop_recursive_item_eval();
        let pop_key = pop_recursive_key_eval();
        let pop_validate = pop_recursive_validate();
        quote! {
            #pop_item
            #pop_key
            #pop_validate
        }
    } else {
        quote! {}
    };
    let root_dynamic_validate_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|(anchor, validate_name, _, _)| {
            let validate_ident = format_ident!("{}", validate_name);
            push_dynamic_validate(anchor, &validate_ident)
        })
        .collect();
    let root_dynamic_key_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|(anchor, _, key_eval_name, _)| {
            let key_eval_ident = format_ident!("{}", key_eval_name);
            push_dynamic_key_eval(anchor, &key_eval_ident)
        })
        .collect();
    let root_dynamic_item_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|(anchor, _, _, item_eval_name)| {
            let item_eval_ident = format_ident!("{}", item_eval_name);
            push_dynamic_item_eval(anchor, &item_eval_ident)
        })
        .collect();
    let root_dynamic_binding_count = root_dynamic_bindings.len();
    let dynamic_push = if dynamic_stack_needed {
        quote! {
            #(#root_dynamic_validate_pushes)*
            #(#root_dynamic_key_pushes)*
            #(#root_dynamic_item_pushes)*
        }
    } else {
        quote! {}
    };
    let dynamic_pop = if dynamic_stack_needed {
        let pop_items = pop_dynamic_item_eval_n(root_dynamic_binding_count);
        let pop_keys = pop_dynamic_key_eval_n(root_dynamic_binding_count);
        let pop_validate = pop_dynamic_validate_n(root_dynamic_binding_count);
        quote! {
            #pop_items
            #pop_keys
            #pop_validate
        }
    } else {
        quote! {}
    };
    let is_valid_body = if recursive_stack_needed || dynamic_stack_needed {
        quote! {
            #recursive_push
            #dynamic_push
            let __result = { #validation_expr };
            #dynamic_pop
            #recursive_pop
            __result
        }
    } else {
        quote! { #validation_expr }
    };

    quote! {
        #[doc(hidden)]
        #[allow(non_snake_case, dead_code, unused_variables, clippy::all)]
        mod #impl_mod_name {
            use super::*;
            #runtime_crate_alias

            #recursive_stack
            #(#regex_helpers)*
            #(#is_valid_helpers)*
            #(#eval_helpers)*
            #(#item_eval_helpers)*

            pub(super) fn is_valid(instance: &#value_ty) -> bool {
                #recompile_trigger
                #is_valid_body
            }
        }

        impl #name {
            pub fn is_valid(instance: &#value_ty) -> bool {
                #impl_mod_name::is_valid(instance)
            }
        }
    }
}
