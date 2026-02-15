use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::context::{CompileContext, PatternEngineConfig};

use super::{
    helpers::DynamicAnchorBinding,
    stack_emit::{
        pop_dynamic_item_eval_n, pop_dynamic_iter_errors_e_n, pop_dynamic_key_eval_n,
        pop_dynamic_validate_n, pop_dynamic_validate_v_n, pop_recursive_item_eval,
        pop_recursive_iter_errors_e, pop_recursive_key_eval, pop_recursive_validate,
        pop_recursive_validate_v, push_dynamic_item_eval, push_dynamic_iter_errors_e,
        push_dynamic_key_eval, push_dynamic_validate, push_dynamic_validate_v,
        push_recursive_item_eval, push_recursive_iter_errors_e, push_recursive_key_eval,
        push_recursive_validate, push_recursive_validate_v,
    },
    CompiledExpr,
};

pub(super) fn emit_root_module(
    ctx: &CompileContext<'_>,
    runtime_crate_alias: Option<&TokenStream>,
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
        .is_valid_helpers
        .iter_bodies()
        .map(|(fname, body)| {
            let func_ident = format_ident!("{}", fname);
            let v_ident = format_ident!("{}_v", fname);
            let e_ident = format_ident!("{}_e", fname);
            let validate_iter = if let Some((validate_body, iter_errors_body)) =
                ctx.is_valid_helpers.get_validate_iter_bodies(fname)
            {
                quote! {
                    #[inline]
                    fn #v_ident<'__i>(
                        instance: &'__i #value_ty,
                        __path: jsonschema::paths::Location,
                    ) -> Option<jsonschema::ValidationError<'__i>> {
                        #validate_body
                    }
                    #[inline]
                    fn #e_ident<'__i>(
                        instance: &'__i #value_ty,
                        __path: jsonschema::paths::Location,
                        __errors: &mut Vec<jsonschema::ValidationError<'__i>>,
                    ) {
                        #iter_errors_body
                    }
                }
            } else {
                // Fallback stubs: delegate to the bool form.  These are generated for
                // recursive/dynamic helpers (which don't store proper validate/iter_errors
                // bodies) so that any cycle-breaking $ref that emitted calls to _v/_e
                // still compiles.  The error reporting is coarse (false_schema at the
                // root path) but correct.
                quote! {
                    #[inline]
                    fn #v_ident<'__i>(
                        instance: &'__i #value_ty,
                        __path: jsonschema::paths::Location,
                    ) -> Option<jsonschema::ValidationError<'__i>> {
                        if !#func_ident(instance) {
                            return Some(jsonschema::keywords_helpers::error::false_schema(
                                "", __path, instance,
                            ));
                        }
                        None
                    }
                    #[inline]
                    fn #e_ident<'__i>(
                        instance: &'__i #value_ty,
                        __path: jsonschema::paths::Location,
                        __errors: &mut Vec<jsonschema::ValidationError<'__i>>,
                    ) {
                        if !#func_ident(instance) {
                            __errors.push(jsonschema::keywords_helpers::error::false_schema(
                                "", __path, instance,
                            ));
                        }
                    }
                }
            };
            quote! {
                #[inline]
                fn #func_ident(instance: &#value_ty) -> bool { #body }
                #validate_iter
            }
        })
        .collect();

    let eval_helpers: Vec<TokenStream> = ctx
        .key_eval_helpers
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

    let item_eval_helpers: Vec<TokenStream> = ctx
        .item_eval_helpers
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
                Vec<(for<'__r> fn(&'__r #value_ty, jsonschema::paths::Location) -> Option<jsonschema::ValidationError<'__r>>, bool)>
            > = std::cell::RefCell::new(Vec::new());
            static __JSONSCHEMA_RECURSIVE_ITER_ERRORS_STACK: std::cell::RefCell<
                Vec<(for<'__r> fn(&'__r #value_ty, jsonschema::paths::Location, &mut Vec<jsonschema::ValidationError<'__r>>), bool)>
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
                Vec<(&'static str, for<'__r> fn(&'__r #value_ty, jsonschema::paths::Location) -> Option<jsonschema::ValidationError<'__r>>)>
            > = std::cell::RefCell::new(Vec::new());
            static __JSONSCHEMA_DYNAMIC_ITER_ERRORS_STACK: std::cell::RefCell<
                Vec<(&'static str, for<'__r> fn(&'__r #value_ty, jsonschema::paths::Location, &mut Vec<jsonschema::ValidationError<'__r>>))>
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
        .map(|b| {
            let validate_ident = format_ident!("{}", b.validate_name);
            push_dynamic_validate(&b.anchor, &validate_ident)
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
    // Validate/iter_errors versions of root dynamic bindings for the parallel stacks.
    let root_dynamic_v_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|b| {
            let v_ident = format_ident!("{}_v", b.validate_name);
            push_dynamic_validate_v(&b.anchor, &v_ident)
        })
        .collect();
    let root_dynamic_e_pushes: Vec<_> = root_dynamic_bindings
        .iter()
        .map(|b| {
            let e_ident = format_ident!("{}_e", b.validate_name);
            push_dynamic_iter_errors_e(&b.anchor, &e_ident)
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

    let validate_stmts = validation_expr.validate.as_ts();
    let iter_errors_stmts = validation_expr.iter_errors.as_ts();

    let validate_fns = {
        let validate_v_ident = format_ident!("validate_v");
        let validate_e_ident = format_ident!("validate_e");
        let validate_body = if recursive_stack_needed || dynamic_stack_needed {
            // Push to bool recursive stack (for sub-calls that use bool form).
            // Also push validate_v to the _v recursive stack so $recursiveRef dispatch
            // in validate() context finds the correct _v function.
            let push_rec_v = if recursive_stack_needed {
                push_recursive_validate_v(&validate_v_ident, root_recursive_anchor)
            } else {
                quote! {}
            };
            let pop_rec_v = if recursive_stack_needed {
                pop_recursive_validate_v()
            } else {
                quote! {}
            };
            let push_dyn_v = if dynamic_stack_needed {
                quote! { #(#root_dynamic_v_pushes)* }
            } else {
                quote! {}
            };
            let pop_dyn_v = if dynamic_stack_needed {
                pop_dynamic_validate_v_n(root_dynamic_binding_count)
            } else {
                quote! {}
            };
            // validate_stmts may contain early `return Some(...)`, so wrap in an IIFE
            // to ensure the stack is always popped even on early return.
            quote! {
                #recursive_push
                #push_rec_v
                #dynamic_push
                #push_dyn_v
                let __result = (|| -> Option<jsonschema::ValidationError<'__i>> {
                    #validate_stmts
                    None
                })();
                #pop_dyn_v
                #dynamic_pop
                #pop_rec_v
                #recursive_pop
                __result
            }
        } else {
            quote! { #validate_stmts None }
        };
        let iter_errors_body = if recursive_stack_needed || dynamic_stack_needed {
            // Similar but push validate_e to the _e recursive/dynamic stacks.
            let push_rec_e = if recursive_stack_needed {
                push_recursive_iter_errors_e(&validate_e_ident, root_recursive_anchor)
            } else {
                quote! {}
            };
            let pop_rec_e = if recursive_stack_needed {
                pop_recursive_iter_errors_e()
            } else {
                quote! {}
            };
            let push_dyn_e = if dynamic_stack_needed {
                quote! { #(#root_dynamic_e_pushes)* }
            } else {
                quote! {}
            };
            let pop_dyn_e = if dynamic_stack_needed {
                pop_dynamic_iter_errors_e_n(root_dynamic_binding_count)
            } else {
                quote! {}
            };
            quote! {
                #recursive_push
                #push_rec_e
                #dynamic_push
                #push_dyn_e
                { #iter_errors_stmts }
                #pop_dyn_e
                #dynamic_pop
                #pop_rec_e
                #recursive_pop
            }
        } else {
            quote! { #iter_errors_stmts }
        };
        quote! {
            pub(super) fn validate_v<'__i>(
                instance: &'__i #value_ty,
                __path: jsonschema::paths::Location,
            ) -> Option<jsonschema::ValidationError<'__i>> {
                #validate_body
            }

            pub(super) fn validate_e<'__i>(
                instance: &'__i #value_ty,
                __path: jsonschema::paths::Location,
                __errors: &mut Vec<jsonschema::ValidationError<'__i>>,
            ) {
                #iter_errors_body
            }
        }
    };

    let validate_impls = quote! {
        pub fn validate<'__i>(
            instance: &'__i #value_ty,
        ) -> ::std::result::Result<(), jsonschema::ValidationError<'__i>> {
            match #impl_mod_name::validate_v(instance, jsonschema::paths::Location::new()) {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }

        pub fn iter_errors<'__i>(
            instance: &'__i #value_ty,
        ) -> impl Iterator<Item = jsonschema::ValidationError<'__i>> {
            let mut __errors = Vec::new();
            #impl_mod_name::validate_e(
                instance,
                jsonschema::paths::Location::new(),
                &mut __errors,
            );
            __errors.into_iter()
        }
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
