use proc_macro2::TokenStream;
use quote::quote;

/// A compiled schema check carrying both output shapes: a boolean `is_valid`
/// expression and a first-error `validate` statement block.
#[derive(Clone)]
pub(crate) struct CompiledExpr {
    pub(crate) is_valid: IsValidExpr,
    pub(crate) validate: ValidateBlock,
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

impl CompiledExpr {
    pub(crate) fn always_true() -> Self {
        Self {
            is_valid: IsValidExpr::AlwaysTrue,
            validate: ValidateBlock::AlwaysValid,
            compile_error: false,
        }
    }

    /// Convert a raw is-valid expression into a `CompiledExpr` that also carries a lazy
    /// `validate` block. The block re-evaluates the check and emits a `false_schema` error
    /// when the check fails. The `schema_path` string is embedded in the generated error.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn from_bool_expr(is_valid: TokenStream, schema_path: &str) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid.clone()),
            validate: ValidateBlock::Expr(quote! {
                if !(#is_valid) {
                    return Some(jsonschema::__private::error::false_schema(
                        #schema_path, __path.into(), instance,
                    ));
                }
            }),
            compile_error: false,
        }
    }

    pub(crate) fn is_compile_error(&self) -> bool {
        self.compile_error
    }

    pub(crate) fn from_error(is_valid: TokenStream) -> Self {
        let mut expr = Self::from_bool_expr(is_valid, "");
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

    /// Construct a `CompiledExpr` with explicit code for `is_valid` and `validate`.
    pub(crate) fn with_validate_blocks(is_valid: TokenStream, validate: TokenStream) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            validate: ValidateBlock::Expr(validate),
            compile_error: false,
        }
    }

    /// Logical AND: simplifies trivially-true arms.
    pub(crate) fn and(self, other: Self) -> Self {
        let compile_error = self.compile_error || other.compile_error;
        let validate = self.validate.and(other.validate);
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
