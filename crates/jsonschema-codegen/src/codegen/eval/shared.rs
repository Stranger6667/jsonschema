use proc_macro2::TokenStream;
use quote::quote;

pub(in crate::codegen) fn compile_guarded_eval(
    value_ty: &TokenStream,
    valid_expr: &TokenStream,
    eval_expr: &TokenStream,
) -> TokenStream {
    quote! {
        (|instance: &#value_ty| #valid_expr)(instance) && (#eval_expr)
    }
}

pub(in crate::codegen) fn compile_one_of_evaluated(
    value_ty: &TokenStream,
    cases: &[(TokenStream, TokenStream)],
) -> Option<TokenStream> {
    if cases.is_empty() {
        return None;
    }

    let valid_exprs: Vec<_> = cases.iter().map(|(valid, _)| valid).collect();
    let eval_exprs: Vec<_> = cases.iter().map(|(_, eval)| eval).collect();

    Some(quote! {
        {
            let mut __one_of_matches = 0usize;
            let mut __one_of_evaluates = false;
            #(
                if (|instance: &#value_ty| #valid_exprs)(instance) {
                    __one_of_matches += 1;
                    __one_of_evaluates = __one_of_evaluates || (#eval_exprs);
                }
            )*
            __one_of_matches == 1 && __one_of_evaluates
        }
    })
}

pub(in crate::codegen) fn compile_if_then_else_evaluated(
    value_ty: &TokenStream,
    if_valid_expr: &TokenStream,
    if_eval_expr: &TokenStream,
    then_eval_expr: &TokenStream,
    else_eval_expr: &TokenStream,
) -> TokenStream {
    quote! {
        if (|instance: &#value_ty| #if_valid_expr)(instance) {
            (#if_eval_expr) || (#then_eval_expr)
        } else {
            #else_eval_expr
        }
    }
}
