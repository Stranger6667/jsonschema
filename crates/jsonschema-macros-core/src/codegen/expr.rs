use proc_macro2::TokenStream;
use quote::quote;

/// A compiled check in three shapes: `is_valid` (bool), `validate` (first error), `collect` (all errors).
#[derive(Clone)]
pub(crate) struct CompiledExpr {
    pub(crate) is_valid: IsValidExpr,
    pub(crate) validate: ValidateBlock,
    pub(crate) collect: CollectBlock,
    pub(crate) compile_error: bool,
}

#[derive(Clone)]
pub(crate) enum IsValidExpr {
    /// The schema always accepts every instance (e.g. `{}` or `true`).
    AlwaysTrue,
    /// A non-trivial boolean expression.
    Expr(TokenStream),
}

/// A statement-sequence code shape for `validate()` mode.
#[derive(Clone)]
pub(crate) enum ValidateBlock {
    /// The keyword imposes no constraint: emit no statements.
    AlwaysValid,
    /// A statement sequence that may return an error.
    Expr(TokenStream),
}

impl ValidateBlock {
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
            (Self::Expr(a), Self::Expr(b)) => Self::Expr(quote! { #a #b }),
        }
    }
}

/// Statement block for `collect`/`iter_errors` mode: pushes errors into `__errors`, never returns early.
#[derive(Clone)]
pub(crate) enum CollectBlock {
    AlwaysValid,
    Expr(TokenStream),
}

impl CollectBlock {
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
            (Self::Expr(a), Self::Expr(b)) => Self::Expr(quote! { #a #b }),
        }
    }
}

/// Derive `collect` from a `validate` block: run it in a closure and push the single error, if any.
/// Only sound for single-error leaves; multi-error loops/branches supply an explicit collect block.
fn derive_collect(validate: &TokenStream) -> CollectBlock {
    CollectBlock::Expr(quote! {
        __errors.extend((|| -> Option<__VE<'_>> { #validate None })());
    })
}

impl CompiledExpr {
    pub(crate) fn always_true() -> Self {
        Self {
            is_valid: IsValidExpr::AlwaysTrue,
            validate: ValidateBlock::AlwaysValid,
            collect: CollectBlock::AlwaysValid,
            compile_error: false,
        }
    }

    /// Convert a raw is-valid expression into a `CompiledExpr` that also carries a lazy
    /// `validate` block. The block re-evaluates the check and emits a `false_schema` error
    /// when the check fails. The `schema_path` string is embedded in the generated error.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn from_bool_expr(is_valid: TokenStream, schema_path: &str) -> Self {
        let validate = quote! {
            if !(#is_valid) {
                return Some(__err::false_schema(
                    #schema_path, __path.into(), instance,
                ));
            }
        };
        Self {
            is_valid: IsValidExpr::Expr(is_valid.clone()),
            collect: derive_collect(&validate),
            validate: ValidateBlock::Expr(validate),
            compile_error: false,
        }
    }

    pub(crate) fn is_compile_error(&self) -> bool {
        self.compile_error
    }

    pub(crate) fn from_error(is_valid: TokenStream) -> Self {
        let mut expr = Self::from_bool_expr(is_valid, "");
        expr.collect = CollectBlock::AlwaysValid;
        expr.compile_error = true;
        expr
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

    /// Single-error leaf from a check and its error: `validate` returns the error, `collect` pushes
    /// it. Emits no closure, unlike [`Self::with_validate_blocks`] deriving `collect` from an opaque
    /// `validate`.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn from_check_and_error(is_valid: TokenStream, error: TokenStream) -> Self {
        Self {
            collect: CollectBlock::Expr(quote! {
                if !(#is_valid) {
                    __errors.push(#error);
                }
            }),
            validate: ValidateBlock::Expr(quote! {
                if !(#is_valid) {
                    return Some(#error);
                }
            }),
            is_valid: IsValidExpr::Expr(is_valid),
            compile_error: false,
        }
    }

    /// Explicit `is_valid`/`validate`, deriving `collect` from `validate` (single-error leaves only;
    /// child-splicing loops/branches use [`Self::with_validate_and_collect_blocks`]).
    pub(crate) fn with_validate_blocks(is_valid: TokenStream, validate: TokenStream) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            collect: derive_collect(&validate),
            validate: ValidateBlock::Expr(validate),
            compile_error: false,
        }
    }

    /// Explicit code for all three modes; for multi-error `validate` blocks whose `collect` must
    /// accumulate every error, not just the first.
    pub(crate) fn with_validate_and_collect_blocks(
        is_valid: TokenStream,
        validate: TokenStream,
        collect: TokenStream,
    ) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            validate: ValidateBlock::Expr(validate),
            collect: CollectBlock::Expr(collect),
            compile_error: false,
        }
    }

    /// Logical AND: simplifies trivially-true arms.
    pub(crate) fn and(self, other: Self) -> Self {
        let compile_error = self.compile_error || other.compile_error;
        let validate = self.validate.and(other.validate);
        let collect = self.collect.and(other.collect);
        let is_valid = match (self.is_valid, other.is_valid) {
            (IsValidExpr::AlwaysTrue, b) => b,
            (a, IsValidExpr::AlwaysTrue) => a,
            (IsValidExpr::Expr(a), IsValidExpr::Expr(b)) => {
                IsValidExpr::Expr(quote! { (#a) && (#b) })
            }
        };
        Self {
            is_valid,
            validate,
            collect,
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
        Self::from_bool_expr(tokens, "")
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
