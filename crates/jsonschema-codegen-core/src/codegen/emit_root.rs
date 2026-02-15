use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::context::{CompileContext, PatternEngineConfig};

use super::{
    helpers::DynamicAnchorBinding,
    stack_emit::{
        pop_dynamic_is_valid_n, pop_dynamic_item_eval_n, pop_dynamic_key_eval_n,
        pop_dynamic_validate_n, pop_recursive_is_valid, pop_recursive_item_eval,
        pop_recursive_key_eval, pop_recursive_validate, push_dynamic_is_valid,
        push_dynamic_item_eval, push_dynamic_key_eval, push_dynamic_validate,
        push_recursive_is_valid, push_recursive_item_eval, push_recursive_key_eval,
        push_recursive_validate,
    },
    CompiledExpr,
};

pub(super) fn emit_root_module(
    ctx: &CompileContext<'_>,
    runtime_crate: Option<&TokenStream>,
    recompile_trigger: &TokenStream,
    name: &Ident,
    impl_mod_name: &Ident,
    validation_expr: &CompiledExpr,
    recursive_stack_needed: bool,
    dynamic_stack_needed: bool,
    root_recursive_anchor: bool,
    root_key_eval_ident: Option<&Ident>,
    root_item_eval_ident: Option<&Ident>,
    root_dynamic_bindings: &[DynamicAnchorBinding],
) -> TokenStream {
    // The `impl` block is outside the aliased module, so it needs the full crate path.
    let runtime_crate_use = runtime_crate.map(|path| quote! { use #path as jsonschema; });
    let runtime_crate = runtime_crate
        .cloned()
        .unwrap_or_else(|| quote! { jsonschema });
    let ref_cycle_needed = ctx.uses_ref_cycle;
    let value_ty = crate::codegen::emit_serde::value_ty();
    let map_ty = crate::codegen::emit_serde::map_ty();
    let value_slice_ty = crate::codegen::emit_serde::value_slice_ty();

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

    let mut uri_cache_statics: Vec<TokenStream> = Vec::new();
    let mut uri_cache_wrappers: Vec<TokenStream> = Vec::new();
    let mut uri_cache_clears: Vec<TokenStream> = Vec::new();
    for cache in &ctx.uri_format_caches {
        let static_ident =
            format_ident!("__JSONSCHEMA_{}_FORMAT_CACHE", cache.base().to_uppercase());
        let wrapper_ident = format_ident!("__cached_{}", cache.base());
        let format_fn = format_ident!("{}", cache.format_fn());
        uri_cache_statics.push(quote! {
            static #static_ident: std::cell::RefCell<jsonschema::__private::format::Cache> =
                std::cell::RefCell::new(jsonschema::__private::format::Cache::default());
        });
        uri_cache_wrappers.push(quote! {
            #[inline]
            fn #wrapper_ident(s: &str) -> bool {
                #static_ident.with_borrow_mut(|cache| {
                    if let Some(&valid) = cache.get(s) {
                        return valid;
                    }
                    let valid = jsonschema::__private::format::#format_fn(s);
                    cache.insert(Box::from(s), valid);
                    valid
                })
            }
        });
        uri_cache_clears.push(quote! {
            #static_ident.with_borrow_mut(|cache| cache.clear());
        });
    }
    let uri_cache_defs = if uri_cache_statics.is_empty() {
        quote! {}
    } else {
        quote! {
            std::thread_local! {
                #(#uri_cache_statics)*
            }
            #(#uri_cache_wrappers)*
        }
    };
    let uri_cache_clears = quote! { #(#uri_cache_clears)* };

    let is_valid_fns: Vec<TokenStream> = ctx
        .is_valid_fns
        .iter_bodies()
        .map(|(fname, body)| {
            let func_ident = format_ident!("{}", fname);
            let validate_ident = format_ident!("{}_validate", fname);
            let validate_body = ctx
                .is_valid_fns
                .get_validate_body(fname)
                .expect("every is_valid fn stores a validate body");
            quote! {
                #[inline]
                fn #func_ident(instance: &#value_ty) -> bool { #body }
                #[inline]
                fn #validate_ident<'__i>(
                    instance: &'__i #value_ty,
                    __path: &jsonschema::paths::LazyLocation,
                ) -> Option<jsonschema::ValidationError<'__i>> {
                    #validate_body
                }
            }
        })
        .collect();

    let key_eval_fns: Vec<TokenStream> = ctx
        .key_eval_fns
        .iter_bodies()
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

    let item_eval_fns: Vec<TokenStream> = ctx
        .item_eval_fns
        .iter_bodies()
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
            static __JSONSCHEMA_RECURSIVE_VALIDATE_STACK: std::cell::RefCell<
                Vec<(for<'__r, '__a, '__b, '__c> fn(&'__r #value_ty, &'__c jsonschema::paths::LazyLocation<'__a, '__b>) -> Option<jsonschema::ValidationError<'__r>>, bool)>
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
            static __JSONSCHEMA_DYNAMIC_VALIDATE_STACK: std::cell::RefCell<
                Vec<(&'static str, for<'__r, '__a, '__b, '__c> fn(&'__r #value_ty, &'__c jsonschema::paths::LazyLocation<'__a, '__b>) -> Option<jsonschema::ValidationError<'__r>>)>
            > = std::cell::RefCell::new(Vec::new());
        }
    } else {
        quote! {}
    };
    // Break self-referential `$dynamicRef`/`$recursiveRef` cycles at runtime, keyed by
    // (helper fn pointer, instance pointer). Separate stacks for validation vs eval tracking.
    let mark_defs = if recursive_stack_needed || dynamic_stack_needed || ref_cycle_needed {
        quote! {
            static __JSONSCHEMA_VALIDATE_MARK: std::cell::RefCell<Vec<(usize, usize)>> =
                std::cell::RefCell::new(Vec::new());
            static __JSONSCHEMA_EVAL_MARK: std::cell::RefCell<Vec<(usize, usize)>> =
                std::cell::RefCell::new(Vec::new());
        }
    } else {
        quote! {}
    };
    let recursive_stack = if recursive_stack_needed || dynamic_stack_needed || ref_cycle_needed {
        quote! {
            std::thread_local! {
                #recursive_stack_defs
                #dynamic_stack_defs
                #mark_defs
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
        let push_is_valid = push_recursive_is_valid(&is_valid_ident, root_recursive_anchor);
        let push_key = push_recursive_key_eval(&root_key_eval_ident, root_recursive_anchor);
        let push_item = push_recursive_item_eval(&root_item_eval_ident, root_recursive_anchor);
        quote! {
            #push_is_valid
            #push_key
            #push_item
        }
    } else {
        quote! {}
    };
    let recursive_pop = if recursive_stack_needed {
        let pop_item = pop_recursive_item_eval();
        let pop_key = pop_recursive_key_eval();
        let pop_is_valid = pop_recursive_is_valid();
        quote! {
            #pop_item
            #pop_key
            #pop_is_valid
        }
    } else {
        quote! {}
    };
    let root_dynamic_is_valid_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|b| {
            let is_valid_ident = format_ident!("{}", b.is_valid_name);
            push_dynamic_is_valid(&b.anchor, &is_valid_ident)
        })
        .collect();
    let root_dynamic_key_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|b| {
            let key_eval_ident = format_ident!("{}", b.key_eval_name);
            push_dynamic_key_eval(&b.anchor, &key_eval_ident)
        })
        .collect();
    let root_dynamic_item_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|b| {
            let item_eval_ident = format_ident!("{}", b.item_eval_name);
            push_dynamic_item_eval(&b.anchor, &item_eval_ident)
        })
        .collect();
    let root_dynamic_validate_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|b| {
            let validate_ident = format_ident!("{}_validate", b.is_valid_name);
            push_dynamic_validate(&b.anchor, &validate_ident)
        })
        .collect();
    let root_dynamic_binding_count = root_dynamic_bindings.len();
    let dynamic_push = if dynamic_stack_needed {
        quote! {
            #(#root_dynamic_is_valid_pushes)*
            #(#root_dynamic_key_pushes)*
            #(#root_dynamic_item_pushes)*
        }
    } else {
        quote! {}
    };
    let dynamic_pop = if dynamic_stack_needed {
        let pop_items = pop_dynamic_item_eval_n(root_dynamic_binding_count);
        let pop_keys = pop_dynamic_key_eval_n(root_dynamic_binding_count);
        let pop_is_valid = pop_dynamic_is_valid_n(root_dynamic_binding_count);
        quote! {
            #pop_items
            #pop_keys
            #pop_is_valid
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

    let validate_stmts = validation_expr.validate.as_token_stream();

    let validate_fns = {
        let validate_ident = format_ident!("validate");
        let validate_body = if recursive_stack_needed || dynamic_stack_needed {
            // Push to bool recursive stack (for sub-calls that use bool form).
            // Also push validate to the validate recursive stack so $recursiveRef
            // dispatch in validate() context finds the correct validate function.
            let recursive_validate_push = if recursive_stack_needed {
                push_recursive_validate(&validate_ident, root_recursive_anchor)
            } else {
                quote! {}
            };
            let recursive_validate_pop = if recursive_stack_needed {
                pop_recursive_validate()
            } else {
                quote! {}
            };
            let dynamic_validate_push = if dynamic_stack_needed {
                quote! { #(#root_dynamic_validate_pushes)* }
            } else {
                quote! {}
            };
            let dynamic_validate_pop = if dynamic_stack_needed {
                pop_dynamic_validate_n(root_dynamic_binding_count)
            } else {
                quote! {}
            };
            // validate_stmts may contain early `return Some(...)`, so wrap in an IIFE
            // to ensure the stack is always popped even on early return.
            quote! {
                #recursive_push
                #recursive_validate_push
                #dynamic_push
                #dynamic_validate_push
                let __result = (|| -> Option<jsonschema::ValidationError<'__i>> {
                    #validate_stmts
                    None
                })();
                #dynamic_validate_pop
                #dynamic_pop
                #recursive_validate_pop
                #recursive_pop
                __result
            }
        } else {
            quote! { #validate_stmts None }
        };
        quote! {
            pub(super) fn validate<'__i>(
                instance: &'__i #value_ty,
                __path: &jsonschema::paths::LazyLocation,
            ) -> Option<jsonschema::ValidationError<'__i>> {
                #uri_cache_clears
                #validate_body
            }
        }
    };

    let validate_impls = quote! {
        pub fn validate<'__i>(
            instance: &'__i #value_ty,
        ) -> ::std::result::Result<(), #runtime_crate::ValidationError<'__i>> {
            match #impl_mod_name::validate(instance, &#runtime_crate::paths::LazyLocation::new()) {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
    };

    quote! {
        #[doc(hidden)]
        #[allow(non_snake_case, dead_code, unused_variables, unreachable_code, clippy::all)]
        mod #impl_mod_name {
            use super::*;
            #runtime_crate_use

            #recursive_stack
            #uri_cache_defs
            #(#regex_helpers)*
            #(#is_valid_fns)*
            #(#key_eval_fns)*
            #(#item_eval_fns)*

            pub(super) fn is_valid(instance: &#value_ty) -> bool {
                #recompile_trigger
                #uri_cache_clears
                #is_valid_body
            }

            #validate_fns
        }

        impl #name {
            pub fn is_valid(instance: &#value_ty) -> bool {
                #impl_mod_name::is_valid(instance)
            }

            #validate_impls
        }
    }
}
