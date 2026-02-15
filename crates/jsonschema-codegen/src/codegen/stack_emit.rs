use proc_macro2::{Ident, TokenStream};
use quote::quote;

pub(in crate::codegen) fn push_recursive_validate(
    validate_ident: &Ident,
    is_recursive_anchor: bool,
) -> TokenStream {
    quote! {
        __JSONSCHEMA_RECURSIVE_STACK.with(|stack| {
            stack.borrow_mut().push((#validate_ident, #is_recursive_anchor));
        });
    }
}

pub(in crate::codegen) fn pop_recursive_validate() -> TokenStream {
    quote! {
        __JSONSCHEMA_RECURSIVE_STACK.with(|stack| { stack.borrow_mut().pop(); });
    }
}

pub(in crate::codegen) fn push_recursive_key_eval(
    key_eval_ident: &Ident,
    is_recursive_anchor: bool,
) -> TokenStream {
    quote! {
        __JSONSCHEMA_RECURSIVE_KEY_EVAL_STACK.with(|stack| {
            stack.borrow_mut().push((#key_eval_ident, #is_recursive_anchor));
        });
    }
}

pub(in crate::codegen) fn pop_recursive_key_eval() -> TokenStream {
    quote! {
        __JSONSCHEMA_RECURSIVE_KEY_EVAL_STACK.with(|stack| { stack.borrow_mut().pop(); });
    }
}

pub(in crate::codegen) fn push_recursive_item_eval(
    item_eval_ident: &Ident,
    is_recursive_anchor: bool,
) -> TokenStream {
    quote! {
        __JSONSCHEMA_RECURSIVE_ITEM_EVAL_STACK.with(|stack| {
            stack.borrow_mut().push((#item_eval_ident, #is_recursive_anchor));
        });
    }
}

pub(in crate::codegen) fn pop_recursive_item_eval() -> TokenStream {
    quote! {
        __JSONSCHEMA_RECURSIVE_ITEM_EVAL_STACK.with(|stack| { stack.borrow_mut().pop(); });
    }
}

pub(in crate::codegen) fn push_dynamic_validate(
    anchor: &str,
    validate_ident: &Ident,
) -> TokenStream {
    quote! {
        __JSONSCHEMA_DYNAMIC_STACK.with(|stack| {
            stack.borrow_mut().push((#anchor, #validate_ident));
        });
    }
}

pub(in crate::codegen) fn pop_dynamic_validate_n(count: usize) -> TokenStream {
    quote! {
        for _ in 0..#count {
            __JSONSCHEMA_DYNAMIC_STACK.with(|stack| { stack.borrow_mut().pop(); });
        }
    }
}

pub(in crate::codegen) fn push_dynamic_key_eval(
    anchor: &str,
    key_eval_ident: &Ident,
) -> TokenStream {
    quote! {
        __JSONSCHEMA_DYNAMIC_KEY_EVAL_STACK.with(|stack| {
            stack.borrow_mut().push((#anchor, #key_eval_ident));
        });
    }
}

pub(in crate::codegen) fn pop_dynamic_key_eval_n(count: usize) -> TokenStream {
    quote! {
        for _ in 0..#count {
            __JSONSCHEMA_DYNAMIC_KEY_EVAL_STACK.with(|stack| { stack.borrow_mut().pop(); });
        }
    }
}

pub(in crate::codegen) fn push_dynamic_item_eval(
    anchor: &str,
    item_eval_ident: &Ident,
) -> TokenStream {
    quote! {
        __JSONSCHEMA_DYNAMIC_ITEM_EVAL_STACK.with(|stack| {
            stack.borrow_mut().push((#anchor, #item_eval_ident));
        });
    }
}

pub(in crate::codegen) fn pop_dynamic_item_eval_n(count: usize) -> TokenStream {
    quote! {
        for _ in 0..#count {
            __JSONSCHEMA_DYNAMIC_ITEM_EVAL_STACK.with(|stack| { stack.borrow_mut().pop(); });
        }
    }
}
