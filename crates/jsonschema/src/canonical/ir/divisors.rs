//! The divisors a numeric leaf admits values of.
use super::BoundRational;

/// Divisors every admitted value is a multiple of.
///
/// `members` is every divisor the leaf was built from; `folded` is the same set with each pair one
/// divisor can stand for folded together. Both are kept: folding on the way in would leave a later
/// intersection a divisor short and make the result depend on the order the sets met in, while
/// folding on the way out would repeat the work at every comparison.
#[derive(Debug, Clone, Default)]
pub(crate) struct Divisors {
    members: Vec<BoundRational>,
    folded: Vec<BoundRational>,
}

// Two sets constrain the same values exactly when they fold alike, and the folded form is what a
// leaf is emitted and sorted by, so it is what equality and order read.
impl PartialEq for Divisors {
    fn eq(&self, other: &Self) -> bool {
        self.folded == other.folded
    }
}

impl Eq for Divisors {}

impl std::hash::Hash for Divisors {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.folded.hash(state);
    }
}

impl PartialOrd for Divisors {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Divisors {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.folded.cmp(&other.folded)
    }
}

impl Divisors {
    pub(crate) fn one(step: BoundRational) -> Self {
        Self::from_members(vec![step])
    }

    /// The only way to build a set, so the folded form can never fall behind the members.
    fn from_members(mut members: Vec<BoundRational>) -> Self {
        members.sort();
        members.dedup();
        let folded = fold(members.clone());
        Self { members, folded }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The divisors as a leaf is emitted and compared by.
    pub(crate) fn as_slice(&self) -> &[BoundRational] {
        &self.folded
    }

    /// The divisor standing for the whole set, when one does.
    pub(crate) fn sole(&self) -> Option<&BoundRational> {
        match self.folded.as_slice() {
            [step] => Some(step),
            _ => None,
        }
    }

    /// The divisors admitting exactly the values both sets do.
    pub(crate) fn intersect(mut self, other: Self) -> Self {
        self.members.extend(other.members);
        Self::from_members(self.members)
    }

    /// The divisors still constraining a whole value; the rest every integer already meets.
    pub(crate) fn over_integers(mut self) -> Self {
        self.members.retain(|step| !step.is_vacuous_over_integers());
        Self::from_members(self.members)
    }

    /// Whether the interval holds a value every divisor admits. A folded divisor constrains more
    /// than the ones it came from, so the answer is taken on the folded set.
    pub(crate) fn admit_between(
        &self,
        minimum: Option<&super::BoundNumber>,
        maximum: Option<&super::BoundNumber>,
    ) -> bool {
        self.folded
            .iter()
            .all(|step| step.admits_between(minimum, maximum))
    }

    /// Whether `value` is a multiple of every divisor, as the validator decides it.
    pub(crate) fn divide(&self, value: &serde_json::Number) -> bool {
        self.members.iter().all(|step| step.divides(value))
    }

    /// Whether every value the `other` divisors admit is also a multiple of all of these. Divisors
    /// taking different arithmetic are left incomparable, since neither stands for the other.
    pub(crate) fn divide_all(&self, other: &Self) -> bool {
        self.folded.iter().all(|step| {
            other
                .folded
                .iter()
                .any(|finer| step.shares_arithmetic(finer) && step.divides_divisor(finer))
        })
    }
}

/// Fold pairs one divisor can stand for until none is left. Folding starts from the sorted set and
/// always takes the first foldable pair, so the divisors a leaf was built from decide the result
/// and the order they arrived in does not.
fn fold(mut divisors: Vec<BoundRational>) -> Vec<BoundRational> {
    // Folding drops a divisor and stripping shrinks one, so neither can run forever.
    loop {
        if let Some((left, right, lcm)) = first_foldable_pair(&divisors) {
            divisors[left] = lcm;
            divisors.remove(right);
        } else if let Some((index, stripped)) = first_strippable(&divisors) {
            divisors[index] = stripped;
        } else {
            break;
        }
        divisors.sort();
        divisors.dedup();
    }
    // Every multiple of a whole divisor is whole, so "whole" beside one adds nothing.
    if divisors
        .iter()
        .any(|step| step.admits_only_whole() && !step.is_identity())
    {
        divisors.retain(|step| !step.is_identity());
    }
    divisors
}

/// The first divisor carrying factors another already supplies, in sorted order.
fn first_strippable(divisors: &[BoundRational]) -> Option<(usize, BoundRational)> {
    divisors.iter().enumerate().find_map(|(index, step)| {
        divisors
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .find_map(|(_, other)| step.without_factors_of(other))
            .map(|stripped| (index, stripped))
    })
}

fn first_foldable_pair(divisors: &[BoundRational]) -> Option<(usize, usize, BoundRational)> {
    divisors.iter().enumerate().find_map(|(left, step)| {
        divisors
            .iter()
            .enumerate()
            .skip(left + 1)
            .find_map(|(right, other)| step.checked_lcm(other).map(|lcm| (left, right, lcm)))
    })
}
