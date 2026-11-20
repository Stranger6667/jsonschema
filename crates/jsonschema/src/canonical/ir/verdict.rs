/// Three-valued verdict for "does this schema accept this value?". `Unknown` marks a constraint no
/// checker can decide; resolving it belongs to the caller alone, because the safe reading differs
/// by direction: an unresolved verdict must never narrow the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Accepts,
    Rejects,
    Unknown,
}

/// How to read a facet no checker covers. A validator without a checker skips the keyword, so a
/// demanded facet passes the value and a barred one rejects it; a value set carries no facet of its
/// own, so intersection can express only that reading. Union keeps such a facet undecided instead,
/// because absorbing a branch on a guess drops values a checker would have kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UncheckableFacet {
    Skipped,
    Undecided,
}

impl Verdict {
    pub(crate) fn from_bool(accepted: bool) -> Self {
        if accepted {
            Verdict::Accepts
        } else {
            Verdict::Rejects
        }
    }

    /// Logical AND over three values: a definite rejection wins, and one undecided input leaves
    /// the whole answer undecided.
    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Verdict::Rejects, Verdict::Accepts | Verdict::Rejects | Verdict::Unknown)
            | (Verdict::Accepts | Verdict::Unknown, Verdict::Rejects) => Verdict::Rejects,
            (Verdict::Unknown, Verdict::Accepts | Verdict::Unknown)
            | (Verdict::Accepts, Verdict::Unknown) => Verdict::Unknown,
            (Verdict::Accepts, Verdict::Accepts) => Verdict::Accepts,
        }
    }

    pub(crate) fn all(verdicts: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Verdict::Accepts;
        for verdict in verdicts {
            result = result.and(verdict);
            if result == Verdict::Rejects {
                return Verdict::Rejects;
            }
        }
        result
    }

    /// Logical OR over three values: a definite admission wins, and one undecided input leaves
    /// the whole answer undecided.
    pub(crate) fn any(verdicts: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Verdict::Rejects;
        for verdict in verdicts {
            match verdict {
                Verdict::Accepts => return Verdict::Accepts,
                Verdict::Unknown => result = Verdict::Unknown,
                Verdict::Rejects => {}
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::Verdict;
    use test_case::test_case;

    #[test_case(Verdict::Accepts, Verdict::Accepts, Verdict::Accepts)]
    #[test_case(Verdict::Accepts, Verdict::Unknown, Verdict::Unknown)]
    #[test_case(Verdict::Unknown, Verdict::Accepts, Verdict::Unknown)]
    #[test_case(Verdict::Unknown, Verdict::Unknown, Verdict::Unknown)]
    #[test_case(Verdict::Rejects, Verdict::Accepts, Verdict::Rejects)]
    #[test_case(Verdict::Rejects, Verdict::Unknown, Verdict::Rejects)]
    #[test_case(Verdict::Accepts, Verdict::Rejects, Verdict::Rejects)]
    #[test_case(Verdict::Unknown, Verdict::Rejects, Verdict::Rejects)]
    #[test_case(Verdict::Rejects, Verdict::Rejects, Verdict::Rejects)]
    fn and_truth_table(left: Verdict, right: Verdict, expected: Verdict) {
        assert_eq!(left.and(right), expected);
    }

    #[test_case(true, Verdict::Accepts)]
    #[test_case(false, Verdict::Rejects)]
    fn from_bool_maps(accepted: bool, expected: Verdict) {
        assert_eq!(Verdict::from_bool(accepted), expected);
    }

    #[test_case(vec![], Verdict::Accepts)]
    #[test_case(vec![Verdict::Accepts, Verdict::Accepts], Verdict::Accepts)]
    #[test_case(vec![Verdict::Accepts, Verdict::Unknown], Verdict::Unknown)]
    #[test_case(vec![Verdict::Unknown, Verdict::Rejects], Verdict::Rejects)]
    #[test_case(vec![Verdict::Rejects, Verdict::Unknown], Verdict::Rejects)]
    fn all_folds(verdicts: Vec<Verdict>, expected: Verdict) {
        assert_eq!(Verdict::all(verdicts), expected);
    }

    #[test_case(vec![], Verdict::Rejects)]
    #[test_case(vec![Verdict::Rejects, Verdict::Rejects], Verdict::Rejects)]
    #[test_case(vec![Verdict::Rejects, Verdict::Unknown], Verdict::Unknown)]
    #[test_case(vec![Verdict::Unknown, Verdict::Accepts], Verdict::Accepts)]
    #[test_case(vec![Verdict::Accepts, Verdict::Rejects], Verdict::Accepts)]
    fn any_folds(verdicts: Vec<Verdict>, expected: Verdict) {
        assert_eq!(Verdict::any(verdicts), expected);
    }
}
