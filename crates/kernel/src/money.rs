//! An amount of money with its currency.

use rust_decimal::Decimal;

use crate::currency::Currency;
use crate::error::KernelError;

// The size of the Postgres column `numeric(20,2)`.
const DECIMAL_PLACES: u32 = 2;
const MAX_INTEGER_DIGITS: u32 = 18;
/// One past the largest amount, in absolute value.
const LIMIT: i64 = 10_i64.pow(MAX_INTEGER_DIGITS);

/// An amount with its currency, sized as the Postgres column `numeric(20,2)`: at most 18
/// integer digits, always at two decimal places. It may be negative: a loss, a change.
///
/// The amount prints as a plain decimal with two places (`"1234.50"`), which is its wire
/// form. There is no multiplication, division or conversion here: rounding belongs to
/// whoever owns the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    amount: Decimal,
    currency: Currency,
}

impl Money {
    /// Keeps the amount at two decimal places. An amount with more than two, or with more
    /// than 18 integer digits, is refused, never rounded.
    pub fn new(amount: Decimal, currency: Currency) -> Result<Self, KernelError> {
        if amount.scale() > DECIMAL_PLACES || !fits(amount) {
            return Err(KernelError::InvalidAmount);
        }
        Ok(Self::normalised(amount, currency))
    }

    /// Reads the wire form: an optional `-`, 1 to 18 digits, then optionally `.` and 1 or
    /// 2 digits.
    pub fn parse(amount: &str, currency: &str) -> Result<Self, KernelError> {
        if !is_wire_form(amount) {
            return Err(KernelError::InvalidAmount);
        }
        let amount = Decimal::from_str_exact(amount).map_err(|_| KernelError::InvalidAmount)?;
        Self::new(amount, Currency::new(currency)?)
    }

    pub fn zero(currency: Currency) -> Self {
        Self::normalised(Decimal::ZERO, currency)
    }

    pub fn amount(&self) -> Decimal {
        self.amount
    }

    pub fn currency(&self) -> Currency {
        self.currency
    }

    pub fn is_negative(&self) -> bool {
        self.amount.is_sign_negative()
    }

    pub fn checked_add(self, other: Self) -> Result<Self, KernelError> {
        self.combine(other, Decimal::checked_add)
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, KernelError> {
        self.combine(other, Decimal::checked_sub)
    }

    fn combine(
        self,
        other: Self,
        operation: fn(Decimal, Decimal) -> Option<Decimal>,
    ) -> Result<Self, KernelError> {
        if self.currency != other.currency {
            return Err(KernelError::CurrencyMismatch {
                left: self.currency,
                right: other.currency,
            });
        }
        let amount = operation(self.amount, other.amount)
            .filter(|amount| fits(*amount))
            .ok_or(KernelError::AmountOverflow)?;
        Ok(Self::normalised(amount, self.currency))
    }

    /// For an amount already known to fit.
    fn normalised(mut amount: Decimal, currency: Currency) -> Self {
        amount.rescale(DECIMAL_PLACES);
        // `Decimal` keeps the sign of a zero, and "-0.00" must never reach the wire.
        if amount.is_zero() {
            amount.set_sign_positive(true);
        }
        Self { amount, currency }
    }
}

fn fits(amount: Decimal) -> bool {
    amount.abs() < Decimal::from(LIMIT)
}

// `Decimal`'s own parser also reads a leading `+`, underscores and any number of digits,
// so the form is checked here and only the digits are left to it.
fn is_wire_form(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (integer, fraction) = match unsigned.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (unsigned, None),
    };
    is_digits(integer, MAX_INTEGER_DIGITS)
        && fraction.is_none_or(|fraction| is_digits(fraction, DECIMAL_PLACES))
}

/// From 1 to `max` ASCII digits, and nothing else.
fn is_digits(text: &str, max: u32) -> bool {
    (1..=max as usize).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppError;

    const LARGEST: &str = "999999999999999999.99";

    fn cop(amount: &str) -> Money {
        Money::parse(amount, "COP").unwrap()
    }

    #[test]
    fn parse_reads_the_wire_form_and_keeps_two_places() {
        for (text, printed) in [
            ("0", "0.00"),
            ("7", "7.00"),
            ("007", "7.00"),
            ("1234.5", "1234.50"),
            ("1234.56", "1234.56"),
            ("-1234.56", "-1234.56"),
            ("0.01", "0.01"),
            ("-0.5", "-0.50"),
            (LARGEST, LARGEST),
            ("-999999999999999999.99", "-999999999999999999.99"),
        ] {
            let money = Money::parse(text, "USD").unwrap();
            assert_eq!(money.amount().to_string(), printed, "{text:?}");
            assert_eq!(money.amount().scale(), 2, "{text:?}");
            assert_eq!(money.currency(), Currency::USD);
        }
    }

    #[test]
    fn parse_refuses_what_is_not_the_wire_form() {
        for text in [
            "",
            "-",
            "+1",
            " 1",
            "1 ",
            "1e3",
            "1E3",
            "1.5e2",
            "1.",
            ".5",
            "-.5",
            "1.234",
            "1.2.3",
            "1.-5",
            "1,5",
            "--1",
            "1-",
            "1_000",
            "١٢٣",
            "NaN",
            "abc",
            "0x10",
            "1234567890123456789",
            "1234567890123456789.00",
        ] {
            let error = Money::parse(text, "COP").unwrap_err();
            assert_eq!(error.code(), "INVALID_AMOUNT", "{text:?}");
        }
    }

    #[test]
    fn parse_refuses_a_currency_that_is_not_one() {
        let error = Money::parse("1.00", "cop").unwrap_err();
        assert_eq!(error.code(), "INVALID_CURRENCY");
    }

    #[test]
    fn the_refusal_does_not_repeat_the_input() {
        let message = Money::parse("987654.321", "COP").unwrap_err().to_string();
        assert!(!message.contains("987654"), "{message}");
    }

    #[test]
    fn new_normalises_the_amount_to_two_places() {
        for (amount, printed) in [
            (Decimal::new(5, 0), "5.00"),
            (Decimal::new(15, 1), "1.50"),
            (Decimal::new(-1234, 2), "-12.34"),
        ] {
            let money = Money::new(amount, Currency::COP).unwrap();
            assert_eq!(money.amount().to_string(), printed);
            assert_eq!(money.currency(), Currency::COP);
        }
    }

    #[test]
    fn new_refuses_a_scale_above_two() {
        for amount in [Decimal::new(1, 3), Decimal::new(1500, 3)] {
            let error = Money::new(amount, Currency::COP).unwrap_err();
            assert_eq!(error.code(), "INVALID_AMOUNT", "{amount}");
        }
    }

    #[test]
    fn new_refuses_more_than_eighteen_integer_digits() {
        let ten_to_the_18 = Decimal::new(1_000_000_000_000_000_000, 0);
        for amount in [ten_to_the_18, -ten_to_the_18, Decimal::MAX, Decimal::MIN] {
            let error = Money::new(amount, Currency::COP).unwrap_err();
            assert_eq!(error.code(), "INVALID_AMOUNT", "{amount}");
        }
        let largest = ten_to_the_18 - Decimal::new(1, 2);
        assert_eq!(Money::new(largest, Currency::COP).unwrap(), cop(LARGEST));
    }

    #[test]
    fn zero_is_zero_at_two_places_in_its_currency() {
        let zero = Money::zero(Currency::USD);
        assert_eq!(zero.amount().to_string(), "0.00");
        assert_eq!(zero.currency(), Currency::USD);
        assert!(!zero.is_negative());
    }

    #[test]
    fn only_an_amount_below_zero_is_negative() {
        assert!(cop("-0.01").is_negative());
        assert!(!cop("0.01").is_negative());
        assert!(!cop("0").is_negative());
    }

    #[test]
    fn a_zero_never_carries_a_minus_sign() {
        let negative_zero = -Decimal::ZERO;
        assert_eq!(negative_zero.to_string(), "-0");
        let zeros = [
            cop("-0"),
            cop("-0.00"),
            Money::new(negative_zero, Currency::COP).unwrap(),
            cop("-1.00").checked_add(cop("1.00")).unwrap(),
            cop("-1.00").checked_sub(cop("-1.00")).unwrap(),
        ];
        for zero in zeros {
            assert_eq!(zero.amount().to_string(), "0.00");
            assert!(!zero.is_negative());
            assert_eq!(zero, Money::zero(Currency::COP));
        }
    }

    #[test]
    fn amounts_are_equal_when_value_and_currency_are() {
        assert_eq!(cop("1.5"), cop("1.50"));
        assert_ne!(cop("1.50"), cop("1.51"));
        assert_ne!(cop("1.50"), Money::parse("1.50", "USD").unwrap());
    }

    #[test]
    fn adding_and_subtracting_keep_the_currency_and_the_two_places() {
        let sum = cop("1.50").checked_add(cop("2.25")).unwrap();
        assert_eq!(sum, cop("3.75"));
        let difference = cop("1.50").checked_sub(cop("2.25")).unwrap();
        assert_eq!(difference, cop("-0.75"));
        assert!(difference.is_negative());
        assert_eq!(difference.amount().scale(), 2);
    }

    #[test]
    fn arithmetic_on_two_currencies_is_a_mismatch() {
        let pesos = cop("1.00");
        let dollars = Money::parse("1.00", "USD").unwrap();
        for result in [pesos.checked_add(dollars), pesos.checked_sub(dollars)] {
            let error = result.unwrap_err();
            assert_eq!(error.code(), "CURRENCY_MISMATCH");
            assert_eq!(
                error,
                KernelError::CurrencyMismatch {
                    left: Currency::COP,
                    right: Currency::USD,
                }
            );
        }
    }

    #[test]
    fn a_result_outside_the_range_is_an_overflow() {
        let largest = cop(LARGEST);
        let smallest = cop("-999999999999999999.99");
        let cent = cop("0.01");
        for result in [
            largest.checked_add(cent),
            smallest.checked_sub(cent),
            largest.checked_add(largest),
            smallest.checked_sub(largest),
        ] {
            assert_eq!(result.unwrap_err().code(), "AMOUNT_OVERFLOW");
        }
        assert_eq!(
            largest.checked_sub(cent).unwrap(),
            cop("999999999999999999.98")
        );
        assert_eq!(largest.checked_add(smallest).unwrap(), cop("0"));
    }
}
