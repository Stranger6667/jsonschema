use proc_macro2::TokenStream;
use quote::quote;

/// A compiled schema check that carries code shapes for all output modes simultaneously.
#[derive(Clone)]
pub(crate) struct CompiledExpr {
    pub(crate) is_valid: IsValidExpr,
    pub(crate) validate: ValidateBlock,
    pub(crate) iter_errors: ValidateBlock,
}

#[derive(Clone)]
pub(crate) enum IsValidExpr {
    /// The schema always accepts every instance (e.g. `{}` or `true`).
    AlwaysTrue,
    /// The schema always rejects every instance (e.g. `false`).
    AlwaysFalse,
    /// A non-trivial boolean expression.
    Expr(TokenStream),
}

/// A statement-sequence code shape for `validate()` / `iter_errors()` mode.
#[derive(Clone)]
pub(crate) enum ValidateBlock {
    /// The keyword imposes no constraint: emit no statements.
    AlwaysValid,
    /// A statement sequence that may return/push an error.
    Expr(TokenStream),
}

impl ValidateBlock {
    /// Convert to a `TokenStream`: empty for `AlwaysValid`, the inner stream for `Expr`.
    pub(crate) fn as_ts(&self) -> TokenStream {
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
            iter_errors: ValidateBlock::AlwaysValid,
        }
    }

    pub(crate) fn always_false() -> Self {
        Self {
            is_valid: IsValidExpr::AlwaysFalse,
            validate: ValidateBlock::Expr(quote! {
                let __r = Some(jsonschema::keywords_helpers::error::false_schema(
                    "", __path.clone(), instance,
                ));
                if let Some(__e) = __r { return Some(__e); }
            }),
            iter_errors: ValidateBlock::Expr(quote! {
                __errors.push(jsonschema::keywords_helpers::error::false_schema(
                    "", __path.clone(), instance,
                ));
            }),
        }
    }

    /// Convert a raw is-valid expression into a `CompiledExpr` that also carries lazy
    /// `validate`/`iter_errors` blocks.  The blocks re-evaluate the check and emit a
    /// `false_schema` error when the check fails.  The `schema_path` string is
    /// embedded in the generated error.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn from_bool_expr(is_valid: TokenStream, schema_path: &str) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid.clone()),
            validate: ValidateBlock::Expr(quote! {
                if !(#is_valid) {
                    return Some(jsonschema::keywords_helpers::error::false_schema(
                        #schema_path, __path.clone(), instance,
                    ));
                }
            }),
            iter_errors: ValidateBlock::Expr(quote! {
                if !(#is_valid) {
                    __errors.push(jsonschema::keywords_helpers::error::false_schema(
                        #schema_path, __path.clone(), instance,
                    ));
                }
            }),
        }
    }

    pub(crate) fn is_trivially_true(&self) -> bool {
        matches!(self.is_valid, IsValidExpr::AlwaysTrue)
    }

    pub(crate) fn is_trivially_false(&self) -> bool {
        matches!(self.is_valid, IsValidExpr::AlwaysFalse)
    }

    /// Non-consuming: returns the `is_valid` [`TokenStream`].
    pub(crate) fn is_valid_ts(&self) -> TokenStream {
        match &self.is_valid {
            IsValidExpr::AlwaysTrue => quote! { true },
            IsValidExpr::AlwaysFalse => quote! { false },
            IsValidExpr::Expr(ts) => ts.clone(),
        }
    }

    /// Extract the `is_valid` [`TokenStream`] (consuming self).
    pub(crate) fn into_token_stream(self) -> TokenStream {
        match self.is_valid {
            IsValidExpr::AlwaysTrue => quote! { true },
            IsValidExpr::AlwaysFalse => quote! { false },
            IsValidExpr::Expr(ts) => ts,
        }
    }

    /// Construct a `CompiledExpr` with explicit code for all three output modes.
    pub(crate) fn with_validate_blocks(
        is_valid: TokenStream,
        validate: TokenStream,
        iter_errors: TokenStream,
    ) -> Self {
        Self {
            is_valid: IsValidExpr::Expr(is_valid),
            validate: ValidateBlock::Expr(validate),
            iter_errors: ValidateBlock::Expr(iter_errors),
        }
    }

    /// Logical AND: returns `AlwaysFalse` if either operand is, simplifies trivially-true arms.
    pub(crate) fn and(self, other: Self) -> Self {
        let validate = self.validate.and(other.validate);
        let iter_errors = self.iter_errors.and(other.iter_errors);
        let is_valid = match (self.is_valid, other.is_valid) {
            (IsValidExpr::AlwaysFalse, _) | (_, IsValidExpr::AlwaysFalse) => {
                IsValidExpr::AlwaysFalse
            }
            (IsValidExpr::AlwaysTrue, b) => b,
            (a, IsValidExpr::AlwaysTrue) => a,
            (IsValidExpr::Expr(a), IsValidExpr::Expr(b)) => {
                IsValidExpr::Expr(quote! { (#a) && (#b) })
            }
        };
        Self {
            is_valid,
            validate,
            iter_errors,
        }
    }

    /// Fold an iterator of expressions with AND, short-circuiting on `AlwaysFalse`
    /// and filtering out `AlwaysTrue` operands. An empty iterator yields `AlwaysTrue`.
    pub(crate) fn combine_and(items: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Self::always_true();
        for item in items {
            if item.is_trivially_false() {
                return Self::always_false();
            }
            if !item.is_trivially_true() {
                result = result.and(item);
            }
        }
        result
    }
}

impl From<TokenStream> for CompiledExpr {
    fn from(ts: TokenStream) -> Self {
        Self::from_bool_expr(ts, "")
    }
}

impl quote::ToTokens for CompiledExpr {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match &self.is_valid {
            IsValidExpr::AlwaysTrue => tokens.extend(quote! { true }),
            IsValidExpr::AlwaysFalse => tokens.extend(quote! { false }),
            IsValidExpr::Expr(ts) => tokens.extend(ts.clone()),
        }
    }
}
