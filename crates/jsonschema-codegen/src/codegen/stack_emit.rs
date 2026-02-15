use proc_macro2::{Ident, TokenStream};
use quote::quote;

macro_rules! define_recursive_ops {
    ($stack:ident, $push:ident, $pop:ident) => {
        pub(crate) fn $push(fn_ident: &Ident, is_recursive_anchor: bool) -> TokenStream {
            quote! {
                $stack.with(|stack| {
                    stack.borrow_mut().push((#fn_ident, #is_recursive_anchor));
                });
            }
        }
        pub(crate) fn $pop() -> TokenStream {
            quote! {
                $stack.with(|stack| { stack.borrow_mut().pop(); });
            }
        }
    };
}

macro_rules! define_dynamic_ops {
    ($stack:ident, $push:ident, $pop_n:ident) => {
        pub(crate) fn $push(anchor: &str, fn_ident: &Ident) -> TokenStream {
            quote! {
                $stack.with(|stack| {
                    stack.borrow_mut().push((#anchor, #fn_ident));
                });
            }
        }
        pub(crate) fn $pop_n(count: usize) -> TokenStream {
            quote! {
                for _ in 0..#count {
                    $stack.with(|stack| { stack.borrow_mut().pop(); });
                }
            }
        }
    };
}

define_recursive_ops!(
    __JSONSCHEMA_RECURSIVE_STACK,
    push_recursive_validate,
    pop_recursive_validate
);
define_recursive_ops!(
    __JSONSCHEMA_RECURSIVE_KEY_EVAL_STACK,
    push_recursive_key_eval,
    pop_recursive_key_eval
);
define_recursive_ops!(
    __JSONSCHEMA_RECURSIVE_ITEM_EVAL_STACK,
    push_recursive_item_eval,
    pop_recursive_item_eval
);

define_dynamic_ops!(
    __JSONSCHEMA_DYNAMIC_STACK,
    push_dynamic_validate,
    pop_dynamic_validate_n
);
define_dynamic_ops!(
    __JSONSCHEMA_DYNAMIC_KEY_EVAL_STACK,
    push_dynamic_key_eval,
    pop_dynamic_key_eval_n
);
define_dynamic_ops!(
    __JSONSCHEMA_DYNAMIC_ITEM_EVAL_STACK,
    push_dynamic_item_eval,
    pop_dynamic_item_eval_n
);

// Parallel stacks for validate(_v) / iter_errors(_e) function pointers.
define_recursive_ops!(
    __JSONSCHEMA_RECURSIVE_VALIDATE_STACK,
    push_recursive_validate_v,
    pop_recursive_validate_v
);
define_recursive_ops!(
    __JSONSCHEMA_RECURSIVE_ITER_ERRORS_STACK,
    push_recursive_iter_errors_e,
    pop_recursive_iter_errors_e
);
define_dynamic_ops!(
    __JSONSCHEMA_DYNAMIC_VALIDATE_STACK,
    push_dynamic_validate_v,
    pop_dynamic_validate_v_n
);
define_dynamic_ops!(
    __JSONSCHEMA_DYNAMIC_ITER_ERRORS_STACK,
    push_dynamic_iter_errors_e,
    pop_dynamic_iter_errors_e_n
);
