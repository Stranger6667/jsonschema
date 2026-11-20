use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::quote;

/// A compiled check in two shapes: `is_valid` (bool) and `errors`, which reports into the
/// `__errors` sink for both `validate` (first error) and `iter_errors` (every error).
#[derive(Clone)]
pub(crate) struct CompiledExpr {
    pub(crate) is_valid: IsValidExpr,
    pub(crate) errors: ErrorsBlock,
    pub(crate) compile_error: bool,
    /// The errors block skips accepted instances on its own, through error functions that run
    /// `is_valid` first, so a caller needs no `is_valid` guard before it.
    pub(crate) skips_valid: bool,
    /// The referenced schema whose error function alone reports this check's errors; equal values
    /// emit equal errors blocks wherever they appear.
    pub(crate) ref_errors: Option<RefErrors>,
}

/// A referenced schema's error function, named by its `is_valid` function, and the reference
/// keyword that reaches it.
#[derive(Clone)]
pub(crate) enum RefErrors {
    /// Run on the instance.
    Call(String, &'static str),
    /// Run on every array item.
    EachItem(String, &'static str),
}

#[derive(Clone)]
pub(crate) enum IsValidExpr {
    /// The schema always accepts every instance (e.g. `{}` or `true`).
    AlwaysTrue,
    /// A non-trivial boolean expression.
    Expr(TokenStream),
}

impl IsValidExpr {
    pub(crate) fn into_token_stream(self) -> TokenStream {
        match self {
            Self::AlwaysTrue => quote! { true },
            Self::Expr(ts) => ts,
        }
    }

    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::AlwaysTrue, b) => b,
            (a, Self::AlwaysTrue) => a,
            (Self::Expr(a), Self::Expr(b)) => Self::Expr(quote! { (#a) && (#b) }),
        }
    }
}

/// Statement block reporting errors into the `__errors` sink; returns `true` from the enclosing
/// function once the sink wants no more errors.
#[derive(Clone)]
pub(crate) enum ErrorsBlock {
    /// The keyword imposes no constraint: emit no statements.
    AlwaysValid,
    Expr(TokenStream),
}

impl ErrorsBlock {
    pub(crate) fn as_token_stream(&self) -> TokenStream {
        match self {
            Self::AlwaysValid => quote! {},
            Self::Expr(ts) => ts.clone(),
        }
    }

    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::AlwaysValid, b) => b,
            (a, Self::AlwaysValid) => a,
            // Appending leaves the left block where it is; `quote!` would copy both into a fresh
            // stream, and these are folded, so the accumulator would be recopied on every keyword.
            (Self::Expr(mut a), Self::Expr(b)) => {
                a.extend(b);
                Self::Expr(a)
            }
        }
    }
}

/// Reports `error` into the `__errors` sink, returning once the sink wants no more errors.
pub(crate) fn push_error(error: &TokenStream) -> TokenStream {
    quote! {
        if __errors.push(#error) {
            return true;
        }
    }
}

/// Runs a generated error function, returning once it reports that the sink wants no more errors.
pub(crate) fn call_errors(call: &TokenStream) -> TokenStream {
    quote! {
        if #call {
            return true;
        }
    }
}

/// Run a first-error block in a labelled block and report its error, if any. First-error blocks
/// never nest outside a closure, so the label is never shadowed.
fn report_first_error(first_error: &TokenStream) -> ErrorsBlock {
    let push = push_error(&quote! { __e });
    ErrorsBlock::Expr(quote! {
        if let Some(__e) = '__first_error: { #first_error None } {
            #push
        }
    })
}

/// The predicate `.all`/`.any` takes for `is_valid` over each `instance`: a call `f(instance)` is
/// passed as `f`, which spares a closure per call site.
pub(crate) fn item_predicate(is_valid: &TokenStream) -> TokenStream {
    let mut tokens = is_valid.clone().into_iter();
    if let (Some(TokenTree::Ident(function)), Some(TokenTree::Group(args)), None) =
        (tokens.next(), tokens.next(), tokens.next())
    {
        let mut args_iter = args.stream().into_iter();
        if args.delimiter() == Delimiter::Parenthesis
            && matches!(
                (args_iter.next(), args_iter.next()),
                (Some(TokenTree::Ident(arg)), None) if arg == "instance"
            )
        {
            return quote! { #function };
        }
    }
    quote! { |instance| #is_valid }
}

impl CompiledExpr {
    pub(crate) fn always_true() -> Self {
        Self {
            is_valid: IsValidExpr::AlwaysTrue,
            errors: ErrorsBlock::AlwaysValid,
            compile_error: false,
            skips_valid: true,
            ref_errors: None,
        }
    }

    /// A check reporting a `false_schema` error at `schema_path` when `is_valid` fails.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn from_bool_expr(
        is_valid: TokenStream,
        schema_path: &str,
        err_instance: &TokenStream,
    ) -> Self {
        Self::from_first_error(
            is_valid.clone(),
            quote! {
                if !(#is_valid) {
                    break '__first_error Some(__err::false_schema(
                        #schema_path, __path.into(), #err_instance,
                    ));
                }
            },
        )
    }

    pub(crate) fn is_compile_error(&self) -> bool {
        self.compile_error
    }

    /// A check that cannot compile: `is_valid` carries the compile error, and the generated code
    /// reports nothing.
    pub(crate) fn from_error(is_valid: TokenStream) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            errors: ErrorsBlock::Expr(TokenStream::new()),
            compile_error: true,
            skips_valid: true,
            ref_errors: None,
        }
    }

    pub(crate) fn is_trivially_true(&self) -> bool {
        matches!(self.is_valid, IsValidExpr::AlwaysTrue)
    }

    /// Non-consuming: returns the `is_valid` [`TokenStream`].
    pub(crate) fn is_valid_token_stream(&self) -> TokenStream {
        match &self.is_valid {
            IsValidExpr::AlwaysTrue => quote! { true },
            IsValidExpr::Expr(ts) => ts.clone(),
        }
    }

    /// Extract the `is_valid` [`TokenStream`] (consuming self).
    pub(crate) fn into_token_stream(self) -> TokenStream {
        match self.is_valid {
            IsValidExpr::AlwaysTrue => quote! { true },
            IsValidExpr::Expr(ts) => ts,
        }
    }

    /// Single-error leaf from a check and its error.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn from_check_and_error(is_valid: TokenStream, error: TokenStream) -> Self {
        let push = push_error(&error);
        Self {
            errors: ErrorsBlock::Expr(quote! {
                if !(#is_valid) {
                    #push
                }
            }),
            is_valid: IsValidExpr::Expr(is_valid),
            compile_error: false,
            skips_valid: false,
            ref_errors: None,
        }
    }

    /// Single-error leaf from a block breaking `'__first_error` with `Some(error)` for its first
    /// error; any later error the block could yield is never reported.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn from_first_error(is_valid: TokenStream, first_error: TokenStream) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            errors: report_first_error(&first_error),
            compile_error: false,
            skips_valid: false,
            ref_errors: None,
        }
    }

    /// Explicit `is_valid` and errors block, for checks that may report several errors.
    pub(crate) fn with_errors(is_valid: TokenStream, errors: TokenStream) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            errors: ErrorsBlock::Expr(errors),
            compile_error: false,
            skips_valid: false,
            ref_errors: None,
        }
    }

    /// The check without its errors, and its errors under an always-true `is_valid`, for errors
    /// reported away from where the check runs.
    pub(crate) fn split_errors(self) -> (Self, Self) {
        let check = Self {
            is_valid: self.is_valid,
            errors: ErrorsBlock::AlwaysValid,
            compile_error: self.compile_error,
            skips_valid: true,
            ref_errors: None,
        };
        let errors = Self {
            is_valid: IsValidExpr::AlwaysTrue,
            errors: self.errors,
            compile_error: self.compile_error,
            skips_valid: self.skips_valid,
            ref_errors: self.ref_errors,
        };
        (check, errors)
    }

    pub(crate) fn with_skips_valid(mut self, skips_valid: bool) -> Self {
        self.skips_valid = skips_valid;
        self
    }

    pub(crate) fn with_ref_errors(mut self, ref_errors: RefErrors) -> Self {
        self.ref_errors = Some(ref_errors);
        self
    }

    /// `errors`, the code reporting this check's errors, behind an `is_valid` guard unless the
    /// errors block skips accepted instances on its own.
    pub(crate) fn guard_errors(&self, errors: &TokenStream) -> TokenStream {
        if self.skips_valid {
            errors.clone()
        } else {
            let is_valid = self.is_valid_token_stream();
            quote! {
                if !(#is_valid) {
                    #errors
                }
            }
        }
    }

    /// Logical AND: simplifies trivially-true arms.
    pub(crate) fn and(self, other: Self) -> Self {
        let compile_error = self.compile_error || other.compile_error;
        let skips_valid = self.skips_valid && other.skips_valid;
        let ref_errors = match (&self.errors, &other.errors) {
            (ErrorsBlock::AlwaysValid, _) => other.ref_errors,
            (_, ErrorsBlock::AlwaysValid) => self.ref_errors,
            _ => None,
        };
        let errors = self.errors.and(other.errors);
        let is_valid = self.is_valid.and(other.is_valid);
        Self {
            is_valid,
            errors,
            compile_error,
            skips_valid,
            ref_errors,
        }
    }

    /// Fold an iterator of expressions with AND, filtering out `AlwaysTrue`
    /// operands. An empty iterator yields `AlwaysTrue`.
    pub(crate) fn combine_and(items: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Self::always_true();
        for item in items {
            if !item.is_trivially_true() {
                result = result.and(item);
            }
        }
        result
    }
}

impl From<TokenStream> for CompiledExpr {
    fn from(tokens: TokenStream) -> Self {
        Self::from_bool_expr(tokens, "", &quote! { instance })
    }
}

impl quote::ToTokens for CompiledExpr {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match &self.is_valid {
            IsValidExpr::AlwaysTrue => tokens.extend(quote! { true }),
            IsValidExpr::Expr(ts) => tokens.extend(ts.clone()),
        }
    }
}
