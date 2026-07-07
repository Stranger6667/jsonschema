use referencing::{Draft, Vocabulary};

use crate::context::CompileContext;

/// Draft-dependent keyword and format availability.
pub(crate) trait DraftExt {
    fn supports_adjacent_validation(self) -> bool;
    fn supports_const_keyword(self) -> bool;
    fn supports_dependent_schemas_keyword(self) -> bool;
    fn supports_dependent_required_keyword(self) -> bool;
    fn supports_prefix_items_keyword(self) -> bool;
    fn supports_recursive_ref_keyword(self) -> bool;
    fn supports_if_then_else_keyword(self) -> bool;
    fn supports_dynamic_ref_keyword(self) -> bool;
    fn supports_property_names_keyword(self) -> bool;
    fn supports_content_validation_keywords(self) -> bool;
    fn supports_draft6_plus_formats(self) -> bool;
    fn supports_draft7_plus_formats(self) -> bool;
    fn supports_draft201909_plus_formats(self) -> bool;
    fn supports_contains_keyword(self) -> bool;
    fn supports_contains_bounds_keyword(self) -> bool;
}

impl DraftExt for Draft {
    #[inline]
    fn supports_adjacent_validation(self) -> bool {
        !matches!(self, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
    }

    #[inline]
    fn supports_const_keyword(self) -> bool {
        !matches!(self, Draft::Draft4)
    }

    #[inline]
    fn supports_dependent_schemas_keyword(self) -> bool {
        matches!(
            self,
            Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
        )
    }

    #[inline]
    fn supports_dependent_required_keyword(self) -> bool {
        matches!(
            self,
            Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
        )
    }

    #[inline]
    fn supports_prefix_items_keyword(self) -> bool {
        matches!(self, Draft::Draft202012 | Draft::Unknown)
    }

    #[inline]
    fn supports_recursive_ref_keyword(self) -> bool {
        matches!(self, Draft::Draft201909)
    }

    #[inline]
    fn supports_if_then_else_keyword(self) -> bool {
        matches!(
            self,
            Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
        )
    }

    #[inline]
    fn supports_dynamic_ref_keyword(self) -> bool {
        matches!(self, Draft::Draft202012 | Draft::Unknown)
    }

    #[inline]
    fn supports_property_names_keyword(self) -> bool {
        !matches!(self, Draft::Draft4)
    }

    #[inline]
    fn supports_content_validation_keywords(self) -> bool {
        matches!(self, Draft::Draft6 | Draft::Draft7)
    }

    #[inline]
    fn supports_draft6_plus_formats(self) -> bool {
        matches!(
            self,
            Draft::Draft6
                | Draft::Draft7
                | Draft::Draft201909
                | Draft::Draft202012
                | Draft::Unknown
        )
    }

    #[inline]
    fn supports_draft7_plus_formats(self) -> bool {
        matches!(
            self,
            Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
        )
    }

    #[inline]
    fn supports_draft201909_plus_formats(self) -> bool {
        matches!(
            self,
            Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
        )
    }

    #[inline]
    fn supports_contains_keyword(self) -> bool {
        !matches!(self, Draft::Draft4)
    }

    #[inline]
    fn supports_contains_bounds_keyword(self) -> bool {
        matches!(
            self,
            Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
        )
    }
}

/// Vocabulary-gated availability: depends on the active meta-schema, not the draft alone.
impl CompileContext<'_> {
    fn has_vocabulary(&self, vocabulary: &Vocabulary) -> bool {
        if self.draft < Draft::Draft201909 || vocabulary == &Vocabulary::Core {
            true
        } else {
            self.vocabularies.contains(vocabulary)
        }
    }

    #[inline]
    pub(crate) fn supports_validation_vocabulary(&self) -> bool {
        self.has_vocabulary(&Vocabulary::Validation)
    }

    #[inline]
    pub(crate) fn supports_applicator_vocabulary(&self) -> bool {
        self.has_vocabulary(&Vocabulary::Applicator)
    }

    #[inline]
    pub(crate) fn supports_unevaluated_items(&self) -> bool {
        match self.draft {
            Draft::Draft201909 => self.supports_applicator_vocabulary(),
            Draft::Draft202012 | Draft::Unknown => self.has_vocabulary(&Vocabulary::Unevaluated),
            _ => false,
        }
    }

    #[inline]
    pub(crate) fn supports_unevaluated_properties(&self) -> bool {
        match self.draft {
            Draft::Draft201909 => self.supports_applicator_vocabulary(),
            Draft::Draft202012 | Draft::Unknown => self.has_vocabulary(&Vocabulary::Unevaluated),
            _ => false,
        }
    }
}
