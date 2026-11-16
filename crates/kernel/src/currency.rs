//! The currency of an amount.

use std::fmt;

use crate::error::KernelError;

/// An ISO 4217 alphabetic code: exactly three upper-case ASCII letters. Which currencies
/// Barras offers is data, not this type.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Currency([u8; 3]);

impl Currency {
    pub const COP: Self = Self(*b"COP");
    pub const USD: Self = Self(*b"USD");

    pub fn new(code: &str) -> Result<Self, KernelError> {
        match code.as_bytes() {
            &[first, second, third]
                if [first, second, third].iter().all(u8::is_ascii_uppercase) =>
            {
                Ok(Self([first, second, third]))
            }
            _ => Err(KernelError::InvalidCurrency),
        }
    }

    pub fn as_str(&self) -> &str {
        // Every value holds ASCII letters, so the bytes are always UTF-8. The fallback is
        // there only because the kernel never unwraps; no value reaches it.
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

// By hand: the derived form prints the three bytes as numbers.
impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Currency").field(&self.as_str()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppError;

    #[test]
    fn three_upper_case_ascii_letters_are_a_currency() {
        for code in ["COP", "USD", "EUR", "AAA", "ZZZ"] {
            let currency = Currency::new(code).unwrap();
            assert_eq!(currency.as_str(), code);
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for code in [
            "",
            "cop",
            "Cop",
            "CO",
            "COPX",
            "C0P",
            "CO ",
            " COP",
            "C-P",
            "ÑOP",
            "CÖP",
            "ＣＯＰ",
            "€",
        ] {
            let error = Currency::new(code).unwrap_err();
            assert_eq!(error.code(), "INVALID_CURRENCY", "{code:?}");
        }
    }

    #[test]
    fn the_constants_are_the_currencies_they_name() {
        assert_eq!(Currency::COP, Currency::new("COP").unwrap());
        assert_eq!(Currency::USD, Currency::new("USD").unwrap());
        assert_ne!(Currency::COP, Currency::USD);
    }

    #[test]
    fn the_refusal_does_not_repeat_the_input() {
        let message = Currency::new("xq7").unwrap_err().to_string();
        assert!(!message.contains("xq7"), "{message}");
    }

    #[test]
    fn debug_shows_the_code() {
        assert_eq!(format!("{:?}", Currency::COP), "Currency(\"COP\")");
    }
}
