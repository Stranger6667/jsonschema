use proc_macro2::TokenStream;
use quote::quote;

/// A compiled check in two shapes: `is_valid` (bool) and `errors`, which reports into the
/// `__errors` sink for both `validate` (first error) and `iter_errors` (every error).
#[derive(Clone)]
pub(crate) struct CompiledExpr {
    pub(crate) is_valid: IsValidExpr,
    pub(crate) errors: ErrorsBlock,
    pub(crate) compile_error: bool,
}

#[derive(Clone)]
pub(crate) enum IsValidExpr {
    /// The schema always accepts every instance (e.g. `{}` or `true`).
    AlwaysTrue,
    /// A non-trivial boolean expression.
    Expr(TokenStream),
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
    /// Convert to a `TokenStream`: empty for `AlwaysValid`, the inner stream for `Expr`.
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

/// A first-error block, as [`CompiledExpr::from_first_error`] takes, returning the first error
/// an errors block reports.
pub(crate) fn first_error_of(errors: &TokenStream) -> TokenStream {
    quote! {
        {
            let mut __first = __err::Errors::new(true);
            let _ = (|| -> bool {
                let __errors = &mut __first;
                #errors
                false
            })();
            if let Some(__e) = __first.into_first() {
                return Some(__e);
            }
        }
    }
}

/// Run a first-error block in a closure and report its error, if any.
fn report_first_error(first_error: &TokenStream) -> ErrorsBlock {
    let push = push_error(&quote! { __e });
    ErrorsBlock::Expr(quote! {
        if let Some(__e) = (|| -> Option<__VE<'_>> { #first_error None })() {
            #push
        }
    })
}

impl CompiledExpr {
    pub(crate) fn always_true() -> Self {
        Self {
            is_valid: IsValidExpr::AlwaysTrue,
            errors: ErrorsBlock::AlwaysValid,
            compile_error: false,
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
                    return Some(__err::false_schema(
                        #schema_path, __path.into(), #err_instance,
                    ));
                }
            },
        )
    }

    pub(crate) fn is_compile_error(&self) -> bool {
        self.compile_error
    }

    /// A check that cannot compile: `is_valid` carries the compile error, and there is nothing to
    /// report at runtime.
    pub(crate) fn from_error(is_valid: TokenStream) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            errors: ErrorsBlock::Expr(TokenStream::new()),
            compile_error: true,
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

    /// Single-error leaf from a check and its error. Emits no closure, unlike
    /// [`Self::from_first_error`].
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
        }
    }

    /// Single-error leaf from a block returning `Some(error)` for its first error; any later error
    /// the block could return is never reported.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn from_first_error(is_valid: TokenStream, first_error: TokenStream) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            errors: report_first_error(&first_error),
            compile_error: false,
        }
    }

    /// Explicit `is_valid` and errors block, for checks that may report several errors.
    pub(crate) fn with_errors(is_valid: TokenStream, errors: TokenStream) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            errors: ErrorsBlock::Expr(errors),
            compile_error: false,
        }
    }

    /// Logical AND: simplifies trivially-true arms.
    pub(crate) fn and(self, other: Self) -> Self {
        let compile_error = self.compile_error || other.compile_error;
        let errors = self.errors.and(other.errors);
        let is_valid = match (self.is_valid, other.is_valid) {
            (IsValidExpr::AlwaysTrue, b) => b,
            (a, IsValidExpr::AlwaysTrue) => a,
            (IsValidExpr::Expr(a), IsValidExpr::Expr(b)) => {
                IsValidExpr::Expr(quote! { (#a) && (#b) })
            }
        };
        Self {
            is_valid,
            errors,
            compile_error,
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
