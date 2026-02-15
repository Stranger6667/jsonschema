use referencing::{Draft, Vocabulary};

use crate::context::CompileContext;

#[inline]
pub(crate) fn supports_adjacent_validation(draft: Draft) -> bool {
    !matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
}

#[inline]
pub(crate) fn supports_const_keyword(draft: Draft) -> bool {
    !matches!(draft, Draft::Draft4)
}

#[inline]
pub(crate) fn supports_dependent_schemas_keyword(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn supports_dependent_required_keyword(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn supports_prefix_items_keyword(draft: Draft) -> bool {
    matches!(draft, Draft::Draft202012 | Draft::Unknown)
}

#[inline]
pub(crate) fn supports_recursive_ref_keyword(draft: Draft) -> bool {
    matches!(draft, Draft::Draft201909)
}

#[inline]
pub(crate) fn supports_if_then_else_keyword(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn supports_dynamic_ref_keyword(draft: Draft) -> bool {
    matches!(draft, Draft::Draft202012 | Draft::Unknown)
}

#[inline]
pub(crate) fn supports_property_names_keyword(draft: Draft) -> bool {
    !matches!(draft, Draft::Draft4)
}

#[inline]
pub(crate) fn supports_content_validation_keywords(draft: Draft) -> bool {
    matches!(draft, Draft::Draft6 | Draft::Draft7)
}

#[inline]
pub(crate) fn supports_draft6_plus_formats(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft6 | Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn supports_draft7_plus_formats(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn supports_draft201909_plus_formats(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn supports_contains_keyword(draft: Draft) -> bool {
    !matches!(draft, Draft::Draft4)
}

#[inline]
pub(crate) fn supports_contains_bounds_keyword(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn supports_unevaluated_items_keyword(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn supports_unevaluated_properties_keyword(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
    )
}

#[inline]
pub(crate) fn has_vocabulary(ctx: &CompileContext<'_>, vocabulary: &Vocabulary) -> bool {
    if ctx.draft < Draft::Draft201909 || vocabulary == &Vocabulary::Core {
        true
    } else {
        ctx.vocabularies.contains(vocabulary)
    }
}

#[inline]
pub(crate) fn supports_validation_vocabulary(ctx: &CompileContext<'_>) -> bool {
    has_vocabulary(ctx, &Vocabulary::Validation)
}

#[inline]
pub(crate) fn supports_applicator_vocabulary(ctx: &CompileContext<'_>) -> bool {
    has_vocabulary(ctx, &Vocabulary::Applicator)
}

#[inline]
pub(crate) fn supports_unevaluated_items_keyword_for_context(ctx: &CompileContext<'_>) -> bool {
    if !supports_unevaluated_items_keyword(ctx.draft) {
        return false;
    }
    match ctx.draft {
        Draft::Draft201909 => supports_applicator_vocabulary(ctx),
        Draft::Draft202012 | Draft::Unknown => has_vocabulary(ctx, &Vocabulary::Unevaluated),
        _ => false,
    }
}

#[inline]
pub(crate) fn supports_unevaluated_properties_keyword_for_context(
    ctx: &CompileContext<'_>,
) -> bool {
    if !supports_unevaluated_properties_keyword(ctx.draft) {
        return false;
    }
    match ctx.draft {
        Draft::Draft201909 => supports_applicator_vocabulary(ctx),
        Draft::Draft202012 | Draft::Unknown => has_vocabulary(ctx, &Vocabulary::Unevaluated),
        _ => false,
    }
}
