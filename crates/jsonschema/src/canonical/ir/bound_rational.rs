//! A positive `multipleOf` divisor over the `number` domain.
use std::cmp::Ordering;

use fraction::{BigFraction, Integer};
use jsonschema_value::numeric_check::{divisor_kind, satisfies_multiple_of, DivisorKind};
use num_traits::{One, Zero};
use serde_json::Number;

use super::{normalized_number, BoundInteger};

/// A divisor kept in the spelling the validator will read, so membership matches it exactly. The
/// exact rational alongside it is what the spelling denotes, and is absent when no rational this
/// build can hold spells back to it - membership stands either way, only arithmetic needs it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BoundRational {
    limit: Number,
    value: Option<BigFraction>,
}

/// What a divisor becomes over the integers it admits.
enum IntegerFold {
    /// Every integer is a multiple, so the divisor leaves no trace.
    Vacuous,
    Divisor(BoundInteger),
    /// The integer form would take different arithmetic than the divisor as written.
    Unfaithful,
}

impl BoundRational {
    /// `None` only when the divisor has no `f64` at all, which is where the validator drops the
    /// keyword. The rational is kept when it spells back to the divisor, which is what lets the
    /// exact arithmetic below stand in for the spelling.
    pub(crate) fn new(limit: &Number) -> Option<Self> {
        let limit = normalized_number(limit);
        limit.as_f64()?;
        let value = exact(&limit).filter(|value| decimal(value).as_ref() == Some(&limit));
        Some(Self { limit, value })
    }

    /// The rational the divisor denotes, when this build holds it.
    fn exact_value(&self) -> Option<&BigFraction> {
        self.value.as_ref()
    }

    pub(crate) fn to_number(&self) -> Number {
        self.limit.clone()
    }

    /// Whether every value the divisor admits is whole, however the validator reads it.
    pub(crate) fn admits_only_whole(&self) -> bool {
        matches!(self.kind(), DivisorKind::Whole | DivisorKind::WholeLossy)
    }

    fn kind(&self) -> DivisorKind {
        divisor_kind(&self.limit)
    }

    /// Whether `value` is a multiple, as the validator decides it.
    pub(crate) fn divides(&self, value: &Number) -> bool {
        satisfies_multiple_of(&self.limit, value)
    }

    /// Whether every multiple of `other` is also a multiple of this divisor. Only meaningful for
    /// divisors taking the same arithmetic, which callers check.
    pub(crate) fn divides_divisor(&self, other: &Self) -> bool {
        let (Some(mine), Some(theirs)) = (self.exact_value(), other.exact_value()) else {
            return false;
        };
        (theirs / mine.clone()).denom().is_none_or(One::is_one)
    }

    /// Whether one divisor may stand in for the other. A whole divisor within `f64` keeps integer
    /// modulo in both builds while a fractional one goes through rational division, so unlike kinds
    /// disagree on instances past that precision even under arbitrary precision.
    #[cfg_attr(feature = "arbitrary-precision", allow(clippy::unused_self))]
    pub(crate) fn shares_arithmetic(&self, other: &Self) -> bool {
        #[cfg(feature = "arbitrary-precision")]
        {
            let _ = other;
            true
        }
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.kind() == other.kind()
        }
    }

    /// The smallest value both divisors admit, or `None` when no divisor the validator reads the
    /// same way spells it.
    pub(crate) fn checked_lcm(&self, other: &Self) -> Option<Self> {
        // For reduced fractions `lcm(p/q, r/s)` is `lcm(p, r) / gcd(q, s)`.
        let (mine, theirs) = (self.exact_value()?, other.exact_value()?);
        let numerator = mine.numer()?.lcm(theirs.numer()?);
        let denominator = mine.denom()?.gcd(theirs.denom()?);
        let combined = Self::new(&decimal(&BigFraction::new(numerator, denominator))?)?;
        // A spelling the validator reads as a different number cannot stand for the pair, and nor
        // can one it reads with different arithmetic.
        (combined.exact_value().is_some()
            && combined.shares_arithmetic(self)
            && combined.shares_arithmetic(other))
        .then_some(combined)
    }

    /// Whether any multiple lies within the interval. An open end always leaves room for one.
    pub(crate) fn admits_between(
        &self,
        minimum: Option<&super::BoundNumber>,
        maximum: Option<&super::BoundNumber>,
    ) -> bool {
        let Some(step) = self.exact_value() else {
            return true;
        };
        let (Some(low), Some((maximum, high))) = (
            minimum.and_then(|bound| exact(&bound.to_number())),
            maximum.and_then(|bound| Some((bound, exact(&bound.to_number())?))),
        ) else {
            return true;
        };
        // The first multiple at or above the lower end. Snapping has already pulled an excluded
        // end onto the next multiple wherever it could, and where it could not this leaf is kept
        // rather than called empty.
        let candidate = step * (low / step.clone()).ceil();
        if candidate > high {
            return false;
        }
        maximum.is_inclusive() || candidate < high
    }

    /// This divisor with the factors `other` already supplies removed, or `None` when nothing can
    /// go. Only factors `other` carries at the same power may, or the pair would admit more.
    /// e.g.  6 beside 2^52  =>  3      (the twos are already there)
    ///       4 beside 6     =>  None   (6 has one two, 4 needs both)
    pub(crate) fn without_factors_of(&self, other: &Self) -> Option<Self> {
        let (mine, theirs) = (self.whole()?, other.whole()?);
        // The largest divisor of `mine` built only from primes `theirs` has.
        let (mut shared, mut rest) = (fraction::BigUint::one(), mine.clone());
        loop {
            let common = rest.gcd(theirs);
            if common.is_one() {
                break;
            }
            shared *= &common;
            rest /= &common;
        }
        if shared.is_one() || !(theirs % &shared).is_zero() {
            return None;
        }
        Self::new(&decimal(&BigFraction::new(rest, fraction::BigUint::one()))?)
            .filter(|stripped| stripped.exact_value().is_some())
    }

    /// The divisor as a whole number, when it is one.
    fn whole(&self) -> Option<&fraction::BigUint> {
        let value = self.exact_value()?;
        value.denom()?.is_one().then(|| value.numer()).flatten()
    }

    /// The first multiple at or past `bound` in `direction`, as a bound admitting it. `None` when no
    /// decimal spells that multiple.
    pub(crate) fn multiple_beyond(
        &self,
        bound: &super::BoundNumber,
        direction: super::Round,
    ) -> Option<super::BoundNumber> {
        let step = self.exact_value()?;
        let spelling = bound.to_number();
        let limit = exact(&spelling).filter(|limit| decimal(limit).as_ref() == Some(&spelling))?;
        let steps = &limit / step.clone();
        let steps = match direction {
            super::Round::Up => steps.ceil(),
            super::Round::Down => steps.floor(),
        };
        let mut candidate = step * steps;
        // An end the interval excludes cannot keep a multiple sitting on it.
        if !bound.is_inclusive() && candidate == limit {
            candidate = match direction {
                super::Round::Up => step + candidate,
                super::Round::Down => candidate - step.clone(),
            };
        }
        let snapped = decimal(&candidate)?;
        // A spelling the validator reads as a different number would move the end, not pin it.
        (exact(&snapped).as_ref() == Some(&candidate))
            .then(|| super::BoundNumber::new(&snapped, true))
    }

    /// Whether every value is a multiple, which any other divisor already implies.
    pub(crate) fn is_identity(&self) -> bool {
        self.exact_value().is_some_and(One::is_one)
    }

    /// Whether every whole value is a multiple, which leaves the divisor no work on the integers.
    pub(crate) fn is_vacuous_over_integers(&self) -> bool {
        matches!(self.integer_fold(), IntegerFold::Vacuous)
    }

    /// The divisor as exact integer arithmetic reads it, when that matches the validator.
    pub(crate) fn exact_integer(&self) -> Option<BoundInteger> {
        match self.integer_fold() {
            IntegerFold::Divisor(divisor) => Some(divisor),
            IntegerFold::Vacuous | IntegerFold::Unfaithful => None,
        }
    }

    /// What this divisor becomes over the integers. A whole divisor keeps its own arithmetic; a
    /// fractional one only folds when every integer already qualifies, since the numerator would
    /// otherwise move it onto integer modulo.
    fn integer_fold(&self) -> IntegerFold {
        let Some(numerator) = self
            .exact_value()
            .and_then(BigFraction::numer)
            .and_then(|numerator| numerator.to_string().parse().ok())
            .and_then(|numerator: Number| BoundInteger::from_number(&numerator))
        else {
            return IntegerFold::Unfaithful;
        };
        if numerator.is_one() {
            return IntegerFold::Vacuous;
        }
        // Integer progressions are reasoned about exactly, which past `f64` precision is not how
        // the validator reads the divisor.
        match self.kind() {
            DivisorKind::Whole if numerator.is_exact_in_f64() => IntegerFold::Divisor(numerator),
            DivisorKind::Whole | DivisorKind::WholeLossy | DivisorKind::Fractional => {
                IntegerFold::Unfaithful
            }
        }
    }
}

impl PartialOrd for BoundRational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BoundRational {
    fn cmp(&self, other: &Self) -> Ordering {
        // A divisor with no rational still needs a place in the order, and its spelling is all
        // there is to sort it by.
        self.value
            .cmp(&other.value)
            .then_with(|| self.limit.to_string().cmp(&other.limit.to_string()))
    }
}

/// The exact rational a JSON number denotes.
fn exact(number: &Number) -> Option<BigFraction> {
    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(value) = jsonschema_value::numeric::bignum::try_parse_bigint(number) {
            return Some(BigFraction::from(value));
        }
        if let Some(value) = jsonschema_value::numeric::bignum::try_parse_bigfraction(number) {
            return Some(value);
        }
    }
    number.as_f64().map(BigFraction::from)
}

/// `value` written as a decimal JSON number, or `None` when no finite decimal spells it.
fn decimal(value: &BigFraction) -> Option<Number> {
    let denominator = value.denom()?;
    // Scaling by ten clears one factor of two and one of five, so a finite decimal exists exactly
    // when those are the denominator's only prime factors, and the wider power decides how many
    // places it takes.
    let (mut rest, mut twos, mut fives) = (denominator.clone(), 0_u32, 0_u32);
    let (two, five) = (fraction::BigUint::from(2_u8), fraction::BigUint::from(5_u8));
    while (&rest % &two).is_zero() {
        rest /= &two;
        twos += 1;
    }
    while (&rest % &five).is_zero() {
        rest /= &five;
        fives += 1;
    }
    debug_assert!(
        rest.is_one(),
        "a JSON number's denominator is a power of ten"
    );
    let places = twos.max(fives) as usize;
    format!("{value:.places$}").parse().ok()
}
