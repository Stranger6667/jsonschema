/// Three-valued verdict for "does this schema admit this value?". `Unknown` marks a constraint no
/// checker can decide; resolving it belongs to the caller alone, because the safe reading differs
/// by direction: an unresolved verdict must never narrow the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Admits,
    Rejects,
    Unknown,
}

impl Verdict {
    pub(crate) fn from_bool(admitted: bool) -> Self {
        if admitted {
            Verdict::Admits
        } else {
            Verdict::Rejects
        }
    }

    /// Kleene conjunction: a definite rejection dominates, an undecided input taints the rest.
    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Verdict::Rejects, Verdict::Admits | Verdict::Rejects | Verdict::Unknown)
            | (Verdict::Admits | Verdict::Unknown, Verdict::Rejects) => Verdict::Rejects,
            (Verdict::Unknown, Verdict::Admits | Verdict::Unknown)
            | (Verdict::Admits, Verdict::Unknown) => Verdict::Unknown,
            (Verdict::Admits, Verdict::Admits) => Verdict::Admits,
        }
    }

    pub(crate) fn all(verdicts: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Verdict::Admits;
        for verdict in verdicts {
            result = result.and(verdict);
            if result == Verdict::Rejects {
                return Verdict::Rejects;
            }
        }
        result
    }

    /// Kleene disjunction: a definite admission dominates, an undecided input taints the rest.
    pub(crate) fn any(verdicts: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Verdict::Rejects;
        for verdict in verdicts {
            match verdict {
                Verdict::Admits => return Verdict::Admits,
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

    #[test_case(Verdict::Admits, Verdict::Admits, Verdict::Admits)]
    #[test_case(Verdict::Admits, Verdict::Unknown, Verdict::Unknown)]
    #[test_case(Verdict::Unknown, Verdict::Admits, Verdict::Unknown)]
    #[test_case(Verdict::Unknown, Verdict::Unknown, Verdict::Unknown)]
    #[test_case(Verdict::Rejects, Verdict::Admits, Verdict::Rejects)]
    #[test_case(Verdict::Rejects, Verdict::Unknown, Verdict::Rejects)]
    #[test_case(Verdict::Admits, Verdict::Rejects, Verdict::Rejects)]
    #[test_case(Verdict::Unknown, Verdict::Rejects, Verdict::Rejects)]
    #[test_case(Verdict::Rejects, Verdict::Rejects, Verdict::Rejects)]
    fn and_truth_table(left: Verdict, right: Verdict, expected: Verdict) {
        assert_eq!(left.and(right), expected);
    }

    #[test_case(true, Verdict::Admits)]
    #[test_case(false, Verdict::Rejects)]
    fn from_bool_maps(admitted: bool, expected: Verdict) {
        assert_eq!(Verdict::from_bool(admitted), expected);
    }

    #[test_case(vec![], Verdict::Admits)]
    #[test_case(vec![Verdict::Admits, Verdict::Admits], Verdict::Admits)]
    #[test_case(vec![Verdict::Admits, Verdict::Unknown], Verdict::Unknown)]
    #[test_case(vec![Verdict::Unknown, Verdict::Rejects], Verdict::Rejects)]
    #[test_case(vec![Verdict::Rejects, Verdict::Unknown], Verdict::Rejects)]
    fn all_folds(verdicts: Vec<Verdict>, expected: Verdict) {
        assert_eq!(Verdict::all(verdicts), expected);
    }

    #[test_case(vec![], Verdict::Rejects)]
    #[test_case(vec![Verdict::Rejects, Verdict::Rejects], Verdict::Rejects)]
    #[test_case(vec![Verdict::Rejects, Verdict::Unknown], Verdict::Unknown)]
    #[test_case(vec![Verdict::Unknown, Verdict::Admits], Verdict::Admits)]
    #[test_case(vec![Verdict::Admits, Verdict::Rejects], Verdict::Admits)]
    fn any_folds(verdicts: Vec<Verdict>, expected: Verdict) {
        assert_eq!(Verdict::any(verdicts), expected);
    }
}
