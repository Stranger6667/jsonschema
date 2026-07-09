use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::helpers::DynamicAnchorBinding;

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
            if count == 0 {
                return quote! {};
            }
            quote! {
                $stack.with(|stack| {
                    let mut stack = stack.borrow_mut();
                    for _ in 0..#count {
                        stack.pop();
                    }
                });
            }
        }
    };
}

define_recursive_ops!(
    __JSONSCHEMA_RECURSIVE_STACK,
    push_recursive_is_valid,
    pop_recursive_is_valid
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
    push_dynamic_is_valid,
    pop_dynamic_is_valid_n
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

// Parallel stacks holding validate() function pointers.
define_recursive_ops!(
    __JSONSCHEMA_RECURSIVE_VALIDATE_STACK,
    push_recursive_validate,
    pop_recursive_validate
);
define_dynamic_ops!(
    __JSONSCHEMA_DYNAMIC_VALIDATE_STACK,
    push_dynamic_validate,
    pop_dynamic_validate_n
);

// Parallel stacks holding collect_errors() function pointers.
define_recursive_ops!(
    __JSONSCHEMA_RECURSIVE_COLLECT_STACK,
    push_recursive_collect,
    pop_recursive_collect
);
define_dynamic_ops!(
    __JSONSCHEMA_DYNAMIC_COLLECT_STACK,
    push_dynamic_collect,
    pop_dynamic_collect_n
);

/// Push/pop emitters for one thread-local stack family, plus the accessor
/// selecting which helper-fn name a dynamic-anchor binding contributes.
pub(crate) struct StackFamily {
    pub(crate) push_recursive: fn(&Ident, bool) -> TokenStream,
    pub(crate) pop_recursive: fn() -> TokenStream,
    pub(crate) push_dynamic: fn(&str, &Ident) -> TokenStream,
    pub(crate) pop_dynamic_n: fn(usize) -> TokenStream,
    pub(crate) binding_fn_name: fn(&DynamicAnchorBinding) -> &str,
}

pub(crate) const IS_VALID_FAMILY: StackFamily = StackFamily {
    push_recursive: push_recursive_is_valid,
    pop_recursive: pop_recursive_is_valid,
    push_dynamic: push_dynamic_is_valid,
    pop_dynamic_n: pop_dynamic_is_valid_n,
    binding_fn_name: |binding| &binding.is_valid_name,
};

pub(crate) const KEY_EVAL_FAMILY: StackFamily = StackFamily {
    push_recursive: push_recursive_key_eval,
    pop_recursive: pop_recursive_key_eval,
    push_dynamic: push_dynamic_key_eval,
    pop_dynamic_n: pop_dynamic_key_eval_n,
    binding_fn_name: |binding| &binding.key_eval_name,
};

pub(crate) const ITEM_EVAL_FAMILY: StackFamily = StackFamily {
    push_recursive: push_recursive_item_eval,
    pop_recursive: pop_recursive_item_eval,
    push_dynamic: push_dynamic_item_eval,
    pop_dynamic_n: pop_dynamic_item_eval_n,
    binding_fn_name: |binding| &binding.item_eval_name,
};

impl StackFamily {
    pub(crate) fn dynamic_pushes(&self, bindings: &[DynamicAnchorBinding]) -> Vec<TokenStream> {
        bindings
            .iter()
            .map(|binding| {
                let fn_ident = format_ident!("{}", (self.binding_fn_name)(binding));
                (self.push_dynamic)(&binding.anchor, &fn_ident)
            })
            .collect()
    }
}

/// Wrap `body` in push/pop calls for one stack family. Pops run after the
/// body's value is bound, keeping push/pop balanced on every path.
pub(crate) fn stack_scoped_body(
    family: &StackFamily,
    func_ident: &Ident,
    is_recursive_anchor: bool,
    uses_recursive: bool,
    uses_dynamic: bool,
    bindings: &[DynamicAnchorBinding],
    body: TokenStream,
) -> TokenStream {
    if !uses_recursive && !uses_dynamic {
        return body;
    }
    let recursive_push = if uses_recursive {
        (family.push_recursive)(func_ident, is_recursive_anchor)
    } else {
        TokenStream::default()
    };
    let recursive_pop = uses_recursive
        .then(family.pop_recursive)
        .unwrap_or_default();
    let dynamic_pushes = if uses_dynamic {
        family.dynamic_pushes(bindings)
    } else {
        Vec::new()
    };
    let dynamic_pop = if uses_dynamic {
        (family.pop_dynamic_n)(bindings.len())
    } else {
        TokenStream::default()
    };
    quote! {
        {
            #recursive_push
            #(#dynamic_pushes)*
            let __result = { #body };
            #dynamic_pop
            #recursive_pop
            __result
        }
    }
}
