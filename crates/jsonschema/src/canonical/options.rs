//! Configuration for canonicalization and the entry points that consume it.
//!
//! [`CanonicalizeOptions`] (and its async sibling) carry the draft, registry, retriever, and format/pattern settings,
//! and drive canonicalization via [`document`](crate::canonical::document) intake.

use std::sync::Arc;

#[cfg(feature = "resolve-async")]
use referencing::AsyncRetrieve;
use referencing::{Draft, Registry, Retrieve};
use serde_json::Value;

use crate::{
    canonical::{
        canonicalize_with_resolver,
        context::CanonicalizationContext,
        document::{
            canonical_registry_builder, prepare_root, raw_schema, root_base_uri, PreparedRoot,
        },
        schema::CanonicalSchema,
        CanonicalizationError,
    },
    retriever::DefaultRetriever,
};

/// Build a [`CanonicalizeOptions`] for configurable canonicalization.
#[must_use]
pub fn options() -> CanonicalizeOptions<'static> {
    CanonicalizeOptions::default()
}

/// Build an [`AsyncCanonicalizeOptions`] for configurable async canonicalization.
#[cfg(feature = "resolve-async")]
#[must_use]
pub fn async_options() -> AsyncCanonicalizeOptions<'static> {
    AsyncCanonicalizeOptions::default()
}

/// Configurable canonicalization entry point. Construct via [`options`].
pub struct CanonicalizeOptions<'r> {
    registry: Option<&'r Registry<'r>>,
    retriever: Option<Arc<dyn Retrieve>>,
    pattern_options: crate::options::PatternEngineOptions,
    draft: Option<Draft>,
    validate_formats: Option<bool>,
    base_uri: Option<String>,
    inline_budget: usize,
}

impl Default for CanonicalizeOptions<'_> {
    fn default() -> Self {
        Self {
            registry: None,
            retriever: None,
            pattern_options: crate::options::PatternEngineOptions::default(),
            draft: None,
            validate_formats: None,
            base_uri: None,
            // Inline every resolvable acyclic ref by default: preserves the fully-resolved output `to_json_schema()` consumers depend on.
            inline_budget: usize::MAX,
        }
    }
}

impl<'r> CanonicalizeOptions<'r> {
    /// Cap how much a resolvable acyclic `$ref` may inline.
    ///
    /// A target exceeding `budget` canonical nodes is emitted symbolically as [`CanonicalView::Reference`] in
    /// [`CanonicalSchema::definitions`] rather than inlined. Default `usize::MAX` inlines all; `0` is fully symbolic; cyclic refs are always symbolic.
    ///
    /// [`CanonicalView::Reference`]: crate::canonical::CanonicalView::Reference
    /// [`CanonicalSchema::definitions`]: crate::canonical::CanonicalSchema::definitions
    #[must_use]
    pub fn with_inline_budget(mut self, budget: usize) -> Self {
        self.inline_budget = budget;
        self
    }

    /// Use a pre-built [`Registry`] for external `$ref` resolution.
    ///
    /// Mirrors [`ValidationOptions::with_registry`].
    ///
    /// [`ValidationOptions::with_registry`]: crate::ValidationOptions::with_registry
    #[must_use]
    pub fn with_registry(mut self, registry: &'r Registry<'r>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Resolve relative references against this base URI when the document has no `$id` of its own, or when its root
    /// `$id` is relative.
    ///
    /// Mirrors [`ValidationOptions::with_base_uri`].
    ///
    /// [`ValidationOptions::with_base_uri`]: crate::ValidationOptions::with_base_uri
    #[must_use]
    pub fn with_base_uri(mut self, base_uri: impl Into<String>) -> Self {
        self.base_uri = Some(base_uri.into());
        self
    }

    /// Use a [`Retrieve`] implementation to lazily fetch external schemas.
    ///
    /// Mirrors [`ValidationOptions::with_retriever`].
    ///
    /// [`ValidationOptions::with_retriever`]: crate::ValidationOptions::with_retriever
    #[must_use]
    pub fn with_retriever(mut self, retriever: impl Retrieve + 'static) -> Self {
        self.retriever = Some(Arc::new(retriever));
        self
    }

    /// Use this draft for canonicalization, overriding `$schema` detection.
    #[must_use]
    pub fn with_draft(mut self, draft: Draft) -> Self {
        self.draft = Some(draft);
        self
    }

    /// Set whether canonicalization should treat `format` as a validation assertion.
    ///
    /// Unset mirrors the draft validator default (Draft 4/6/7 assert known formats; 2019-09/2020-12 annotate). Asserting
    /// lets incompatible format intersections like `date`/`uuid` collapse to `false`. Mirrors [`ValidationOptions::should_validate_formats`].
    ///
    /// [`ValidationOptions::should_validate_formats`]: crate::ValidationOptions::should_validate_formats
    #[must_use]
    pub fn should_validate_formats(mut self, enabled: bool) -> Self {
        self.validate_formats = Some(enabled);
        self
    }

    /// Select the regex engine used during canonicalization. For soundness, match the engine the validator will use.
    #[must_use]
    pub fn with_pattern_options<E>(mut self, options: &crate::PatternOptions<E>) -> Self {
        self.pattern_options = options.inner();
        self
    }

    /// Run canonicalization with the configured options.
    ///
    /// # Errors
    ///
    /// Same as [`crate::canonicalize`].
    pub fn canonicalize(self, value: &Value) -> Result<CanonicalSchema, CanonicalizationError> {
        let (draft, validate_formats) = match prepare_root(
            value,
            self.draft,
            self.registry,
            self.validate_formats,
            self.pattern_options,
        )? {
            PreparedRoot::Raw(schema) => return Ok(schema),
            PreparedRoot::Canonicalize {
                draft,
                validate_formats,
            } => (draft, validate_formats),
        };
        let ctx = CanonicalizationContext::with_pattern_options(self.pattern_options)
            .with_draft(draft)
            .with_format_assertions(validate_formats);

        let base_uri = root_base_uri(value, draft, self.base_uri.as_deref());
        let resource = draft.create_resource_ref(value);
        let retriever: Arc<dyn Retrieve> =
            self.retriever.unwrap_or_else(|| Arc::new(DefaultRetriever));
        let prepared = canonical_registry_builder(self.registry, &base_uri, resource, draft)
            .and_then(|builder| builder.retriever(retriever).prepare());
        let Ok(registry) = prepared else {
            return Ok(raw_schema(
                value,
                draft,
                self.pattern_options,
                validate_formats,
            ));
        };
        canonicalize_with_resolver(
            value,
            draft,
            &ctx,
            &registry.resolver(base_uri),
            self.inline_budget,
        )
    }
}

/// Configurable async canonicalization entry point. Construct via [`async_options`].
#[cfg(feature = "resolve-async")]
pub struct AsyncCanonicalizeOptions<'r> {
    registry: Option<&'r Registry<'r>>,
    retriever: Option<Arc<dyn AsyncRetrieve>>,
    pattern_options: crate::options::PatternEngineOptions,
    draft: Option<Draft>,
    validate_formats: Option<bool>,
    base_uri: Option<String>,
    inline_budget: usize,
}

#[cfg(feature = "resolve-async")]
impl Default for AsyncCanonicalizeOptions<'_> {
    fn default() -> Self {
        Self {
            registry: None,
            retriever: None,
            pattern_options: crate::options::PatternEngineOptions::default(),
            draft: None,
            validate_formats: None,
            base_uri: None,
            inline_budget: usize::MAX,
        }
    }
}

#[cfg(feature = "resolve-async")]
impl<'r> AsyncCanonicalizeOptions<'r> {
    /// Cap how much a resolvable acyclic `$ref` may inline.
    ///
    /// Uses the same semantics as [`CanonicalizeOptions::with_inline_budget`].
    #[must_use]
    pub fn with_inline_budget(mut self, budget: usize) -> Self {
        self.inline_budget = budget;
        self
    }

    /// Use a pre-built [`Registry`] for external `$ref` resolution.
    #[must_use]
    pub fn with_registry(mut self, registry: &'r Registry<'r>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Resolve relative references against this base URI when the document has no `$id` of its own, or when its root
    /// `$id` is relative.
    ///
    /// Mirrors [`CanonicalizeOptions::with_base_uri`].
    #[must_use]
    pub fn with_base_uri(mut self, base_uri: impl Into<String>) -> Self {
        self.base_uri = Some(base_uri.into());
        self
    }

    /// Use an [`AsyncRetrieve`] implementation to lazily fetch external schemas.
    #[must_use]
    pub fn with_retriever(mut self, retriever: impl AsyncRetrieve + 'static) -> Self {
        self.retriever = Some(Arc::new(retriever));
        self
    }

    /// Use this draft for canonicalization, overriding `$schema` detection.
    #[must_use]
    pub fn with_draft(mut self, draft: Draft) -> Self {
        self.draft = Some(draft);
        self
    }

    /// Set whether canonicalization should treat `format` as a validation assertion.
    ///
    /// Unset mirrors the draft validator default; asserting lets incompatible intersections like `date`/`uuid` collapse
    /// to `false`. Mirrors [`CanonicalizeOptions::should_validate_formats`].
    #[must_use]
    pub fn should_validate_formats(mut self, enabled: bool) -> Self {
        self.validate_formats = Some(enabled);
        self
    }

    /// Select the regex engine used during canonicalization. For soundness, match the engine the validator will use.
    #[must_use]
    pub fn with_pattern_options<E>(mut self, options: &crate::PatternOptions<E>) -> Self {
        self.pattern_options = options.inner();
        self
    }

    /// Run canonicalization with the configured async options.
    ///
    /// # Errors
    ///
    /// Same as [`crate::canonicalize`].
    pub async fn canonicalize(
        self,
        value: &Value,
    ) -> Result<CanonicalSchema, CanonicalizationError> {
        let (draft, validate_formats) = match prepare_root(
            value,
            self.draft,
            self.registry,
            self.validate_formats,
            self.pattern_options,
        )? {
            PreparedRoot::Raw(schema) => return Ok(schema),
            PreparedRoot::Canonicalize {
                draft,
                validate_formats,
            } => (draft, validate_formats),
        };
        let base_uri = root_base_uri(value, draft, self.base_uri.as_deref());
        let resource = draft.create_resource_ref(value);
        let retriever: Arc<dyn AsyncRetrieve> =
            self.retriever.unwrap_or_else(|| Arc::new(DefaultRetriever));
        // Mirror the sync path: builder construction and async preparation share one raw-schema fallback.
        let prepared = async {
            canonical_registry_builder(self.registry, &base_uri, resource, draft)?
                .async_retriever(retriever)
                .async_prepare()
                .await
        }
        .await;
        let Ok(registry) = prepared else {
            return Ok(raw_schema(
                value,
                draft,
                self.pattern_options,
                validate_formats,
            ));
        };
        // Keep this after `.await`: `CanonicalizationContext` contains non-`Send` cache state.
        let ctx = CanonicalizationContext::with_pattern_options(self.pattern_options)
            .with_draft(draft)
            .with_format_assertions(validate_formats);
        canonicalize_with_resolver(
            value,
            draft,
            &ctx,
            &registry.resolver(base_uri),
            self.inline_budget,
        )
    }
}

#[cfg(all(test, feature = "resolve-async", not(target_arch = "wasm32")))]
mod tests {
    use referencing::{Draft, Registry};
    use serde_json::json;

    use super::async_options;
    use crate::{canonical::CanonicalizationError, PatternOptions};

    #[tokio::test]
    async fn with_draft_overrides_detection() {
        // `prefixItems` is 2020-12-only (ignored under draft-7), so forcing the draft changes the canonical form -
        // proving `with_draft` takes effect rather than being a no-op.
        let schema = json!({"prefixItems": [{"type": "integer"}]});
        let as_2020 = async_options()
            .with_draft(Draft::Draft202012)
            .canonicalize(&schema)
            .await
            .expect("canonicalize");
        let as_draft7 = async_options()
            .with_draft(Draft::Draft7)
            .canonicalize(&schema)
            .await
            .expect("canonicalize");
        assert_ne!(
            as_2020.to_json_schema(),
            as_draft7.to_json_schema(),
            "with_draft must change canonicalization across drafts",
        );
    }

    #[tokio::test]
    async fn should_validate_formats_flag_flows_through() {
        let schema = json!({
            "allOf": [
                {"type": "string", "format": "date"},
                {"type": "string", "format": "uuid"},
            ],
        });
        let asserted = async_options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(true)
            .canonicalize(&schema)
            .await
            .expect("canonicalize");
        assert!(!asserted.is_satisfiable(), "{}", asserted.to_json_schema());

        let annotated = async_options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(false)
            .canonicalize(&schema)
            .await
            .expect("canonicalize");
        assert!(annotated.is_satisfiable());
    }

    #[tokio::test]
    async fn with_pattern_options_canonicalizes() {
        let canonical = async_options()
            .with_pattern_options(&PatternOptions::regex())
            .canonicalize(&json!({"type": "string", "pattern": "^abc$"}))
            .await
            .expect("canonicalize");
        assert!(canonical.is_satisfiable());
    }

    #[tokio::test]
    async fn with_base_uri_resolves_relative_reference() {
        let registry = Registry::new()
            .add("https://example.com/child", json!({"type": "integer"}))
            .expect("valid resource")
            .prepare()
            .expect("registry prepares");
        let canonical = async_options()
            .with_registry(&registry)
            .with_base_uri("https://example.com/main")
            .canonicalize(&json!({"$ref": "child"}))
            .await
            .expect("canonicalize");
        assert_eq!(canonical.to_json_schema()["type"], json!("integer"));
    }

    #[tokio::test]
    async fn non_document_root_is_rejected() {
        let error = async_options()
            .canonicalize(&json!(42))
            .await
            .expect_err("non-object root must error");
        assert!(matches!(error, CanonicalizationError::InvalidSchemaType(_)));
    }
}
