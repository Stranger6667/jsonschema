use serde_json::Number;

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.abs()
}

/// The exact rational a JSON number's decimal form names, reduced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fraction {
    pub(crate) numerator: i128,
    pub(crate) denominator: i128,
}

impl Fraction {
    pub(crate) const ONE: Fraction = Fraction {
        numerator: 1,
        denominator: 1,
    };

    pub(crate) fn integer(value: i128) -> Fraction {
        Fraction {
            numerator: value,
            denominator: 1,
        }
    }

    /// `None` where the value does not fit `i128` arithmetic.
    pub(crate) fn from_number(number: &Number) -> Option<Fraction> {
        let text = number.to_string();
        let (mantissa, exponent) = match text.split_once(['e', 'E']) {
            Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
            None => (text.as_str(), 0),
        };
        let (whole, fractional) = match mantissa.split_once('.') {
            Some((whole, fractional)) => (whole, fractional),
            None => (mantissa, ""),
        };
        let mut numerator = format!("{whole}{fractional}").parse::<i128>().ok()?;
        let mut denominator = 1i128;
        let scale = exponent - i64::try_from(fractional.len()).ok()?;
        if scale >= 0 {
            for _ in 0..scale {
                numerator = numerator.checked_mul(10)?;
            }
        } else {
            for _ in 0..-scale {
                denominator = denominator.checked_mul(10)?;
            }
        }
        let reducer = gcd(numerator, denominator);
        if reducer != 0 {
            numerator /= reducer;
            denominator /= reducer;
        }
        Some(Fraction {
            numerator,
            denominator,
        })
    }

    /// A `multipleOf` value: positive, or nothing.
    pub(crate) fn divisor(number: &Number) -> Option<Fraction> {
        Self::from_number(number).filter(|fraction| fraction.numerator > 0)
    }

    // Overflow reads as "not a multiple"; the wrapper's validator net decides then.
    pub(crate) fn is_multiple_of(self, divisor: Fraction) -> bool {
        let Some(left) = self.numerator.checked_mul(divisor.denominator) else {
            return false;
        };
        let Some(right) = self.denominator.checked_mul(divisor.numerator) else {
            return false;
        };
        right != 0 && left % right == 0
    }

    pub(crate) fn is_integer(self) -> bool {
        self.is_multiple_of(Fraction::ONE)
    }

    /// Multiples of both operands are exactly the multiples of the result.
    pub(crate) fn lcm(self, other: Fraction) -> Option<Fraction> {
        let numerator = self
            .numerator
            .checked_mul(other.numerator / gcd(self.numerator, other.numerator))?;
        let denominator = gcd(self.denominator, other.denominator);
        let reducer = gcd(numerator, denominator);
        Some(Fraction {
            numerator: numerator / reducer,
            denominator: denominator / reducer,
        })
    }

    pub(crate) fn to_f64(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }
}

/// Every barred divisor as a fraction, or `None` when one has no exact reading.
pub(crate) fn excluded_divisors(not_multiple_of: &[Number]) -> Option<Vec<Fraction>> {
    not_multiple_of.iter().map(Fraction::divisor).collect()
}
