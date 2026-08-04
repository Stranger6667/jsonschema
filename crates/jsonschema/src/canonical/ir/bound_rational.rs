//! A positive `multipleOf` divisor over the `number` domain.
use std::cmp::Ordering;

use fraction::{BigFraction, BigInt, Integer, Sign};
use jsonschema_value::numeric_check::{divisor_kind, satisfies_multiple_of, DivisorKind};
use num_traits::{One, Signed, Zero};
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
        let value = denoted(&limit);
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
        // An end no rational this build spells back to is not the end the validator compares
        // against, so the interval is left holding a multiple rather than called empty.
        let (Some(low), Some((maximum, high))) = (
            minimum.and_then(|bound| denoted(&bound.to_number())),
            maximum.and_then(|bound| Some((bound, denoted(&bound.to_number())?))),
        ) else {
            return true;
        };
        // The first multiple at or above the lower end. Snapping has already pulled an excluded
        // end onto the next multiple wherever it could, and where it could not this leaf is kept
        // rather than called empty.
        let Some(candidate) =
            step_index(&low, step, super::Round::Up).and_then(|index| grid_point(step, &index))
        else {
            return true;
        };
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
        let limit = denoted(&bound.to_number())?;
        let mut index = step_index(&limit, step, direction)?;
        let mut candidate = grid_point(step, &index)?;
        // An end the interval excludes cannot keep a multiple sitting on it. One step along the grid
        // is one off the index, which keeps the move clear of rational addition.
        if !bound.is_inclusive() && candidate == limit {
            index = match direction {
                super::Round::Up => index + 1_u8,
                super::Round::Down => index - 1_u8,
            };
            candidate = grid_point(step, &index)?;
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

/// How many whole steps of `divisor` reach `bound`, rounded as `direction` says. Rational division
/// would reduce the quotient before it is rounded away, and reduction walks the whole numerator.
fn step_index(
    bound: &BigFraction,
    divisor: &BigFraction,
    direction: super::Round,
) -> Option<BigInt> {
    let (bound_numerator, bound_denominator) = (bound.numer()?, bound.denom()?);
    let (divisor_numerator, divisor_denominator) = (divisor.numer()?, divisor.denom()?);
    let magnitude = BigInt::from(bound_numerator * divisor_denominator);
    let numerator = if bound.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    };
    let denominator = BigInt::from(bound_denominator * divisor_numerator);
    Some(match direction {
        super::Round::Up => numerator.div_ceil(&denominator),
        super::Round::Down => numerator.div_floor(&denominator),
    })
}

/// `divisor * index`, already in lowest terms. A reduced divisor leaves the index as the only thing
/// its denominator can share a factor with, and taking the index modulo that denominator first keeps
/// the common factor off the numerator, which is the part that grows.
fn grid_point(divisor: &BigFraction, index: &BigInt) -> Option<BigFraction> {
    let (divisor_numerator, divisor_denominator) = (divisor.numer()?, divisor.denom()?);
    let magnitude = index.magnitude();
    let common = (magnitude % divisor_denominator).gcd(divisor_denominator);
    let numerator = divisor_numerator * (magnitude / &common);
    let denominator = divisor_denominator / &common;
    debug_assert!(
        (&numerator % &denominator).gcd(&denominator).is_one(),
        "a reduced divisor times a whole step is already in lowest terms"
    );
    let sign = if index.is_negative() {
        Sign::Minus
    } else {
        Sign::Plus
    };
    Some(BigFraction::new_raw_signed(sign, numerator, denominator))
}

/// The rational a JSON number denotes, when this build reads one that spells back to it. Exact
/// arithmetic may only stand in for a spelling it reproduces: past `f64` precision `exact` rounds,
/// and the rounded value is not what the validator compares against.
fn denoted(number: &Number) -> Option<BigFraction> {
    exact(number).filter(|value| decimal(value).as_ref() == Some(number))
}

/// The rational this build reads a JSON number as, which past `f64` precision is a rounded one.
fn exact(number: &Number) -> Option<BigFraction> {
    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(value) = jsonschema_value::numeric::bignum::try_parse_bigint(number) {
            return Some(whole(value));
        }
        if let Some(value) = jsonschema_value::numeric::bignum::try_parse_bigfraction(number) {
            return Some(value);
        }
    }
    number.as_f64().map(BigFraction::from)
}

/// The rational a whole number denotes. `BigFraction::from` writes the integer out in decimal and
/// reads it back one digit at a time, while a magnitude over one is already the reduced form.
#[cfg(feature = "arbitrary-precision")]
fn whole(value: num_bigint::BigInt) -> BigFraction {
    let (sign, magnitude) = value.into_parts();
    let fraction = BigFraction::new_raw(magnitude, fraction::BigUint::one());
    if sign == num_bigint::Sign::Minus {
        -fraction
    } else {
        fraction
    }
}

/// `value` written as a decimal JSON number, or `None` when no finite decimal spells it.
fn decimal(value: &BigFraction) -> Option<Number> {
    let (numerator, denominator) = (value.numer()?, value.denom()?);
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
    if !rest.is_one() {
        return None;
    }
    let places = twos.max(fives);
    let text = spelled(numerator, denominator, places, value.is_sign_negative());
    debug_assert_eq!(
        text,
        format!("{value:.width$}", width = places as usize),
        "the scaled spelling reads as the formatted one"
    );
    text.parse().ok()
}

/// The text `format!("{value:.places$}")` writes. Scaling to a whole number and placing the point
/// keeps this clear of the fraction crate's formatting, which walks the numerator once per place.
fn spelled(
    numerator: &fraction::BigUint,
    denominator: &fraction::BigUint,
    places: u32,
    negative: bool,
) -> String {
    let scale = fraction::BigUint::from(10_u8).pow(places);
    debug_assert!(
        (&scale % denominator).is_zero(),
        "ten to the places is a multiple of the denominator"
    );
    let digits = (numerator * (scale / denominator)).to_string();
    let places = places as usize;
    let mut text = String::with_capacity(digits.len() + places + 3);
    if negative {
        text.push('-');
    }
    if places == 0 {
        text.push_str(&digits);
        return text;
    }
    if let Some(width) = digits.len().checked_sub(places).filter(|whole| *whole > 0) {
        let (whole, fraction) = digits.split_at(width);
        text.push_str(whole);
        text.push('.');
        text.push_str(fraction);
    } else {
        // Fewer digits than places, so there is no whole part and the rest pads the fraction out.
        text.push_str("0.");
        for _ in 0..places - digits.len() {
            text.push('0');
        }
        text.push_str(&digits);
    }
    text
}
