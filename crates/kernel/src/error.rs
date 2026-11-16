//! The error contract every module implements, and the kernel's own errors.

use thiserror::Error;

use crate::currency::Currency;
use crate::page::PageLimit;

/// What kind of failure an error is, in terms every adapter can map to its own protocol.
/// The domain does not know HTTP: only the HTTP adapter turns a kind into a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Validation,
    Unauthenticated,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    Unavailable,
    Internal,
}

/// The contract of every `<Module>Error`. Its `Display` text is the message for developers.
/// The client receives it verbatim for every kind but `Internal` and `Unavailable`, so it
/// never interpolates a source or a value the caller sent.
pub trait AppError: std::error::Error + Send + Sync + 'static {
    /// The stable `SCREAMING_SNAKE_CASE` code of the failure. It is public API: the app
    /// chooses its text by it, so renaming one is a breaking change.
    fn code(&self) -> &'static str;

    fn kind(&self) -> ErrorKind;
}

/// The failures of the kernel's own types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KernelError {
    #[error("invalid currency: expected three upper-case ASCII letters")]
    InvalidCurrency,
    #[error(
        "invalid amount: expected a decimal with at most 18 integer digits and 2 decimal places"
    )]
    InvalidAmount,
    #[error("invalid id: expected a UUID in its hyphenated lower-case form")]
    InvalidId,
    #[error("invalid page limit: expected a number from 1 to {}", PageLimit::MAX)]
    InvalidPageLimit,
    #[error("invalid cursor: expected 1 to 512 characters, each a letter, a digit, '-' or '_'")]
    InvalidCursor,
    /// The two currencies are for the code that handles the error. The message leaves
    /// them out, like every value a caller passed in.
    #[error("currency mismatch: arithmetic needs two amounts in one currency")]
    CurrencyMismatch { left: Currency, right: Currency },
    #[error("amount overflow: the result does not fit in 18 integer digits")]
    AmountOverflow,
}

impl AppError for KernelError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidCurrency => "INVALID_CURRENCY",
            Self::InvalidAmount => "INVALID_AMOUNT",
            Self::InvalidId => "INVALID_ID",
            Self::InvalidPageLimit => "INVALID_PAGE_LIMIT",
            Self::InvalidCursor => "INVALID_CURSOR",
            Self::CurrencyMismatch { .. } => "CURRENCY_MISMATCH",
            Self::AmountOverflow => "AMOUNT_OVERFLOW",
        }
    }

    fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidCurrency
            | Self::InvalidAmount
            | Self::InvalidId
            | Self::InvalidPageLimit
            | Self::InvalidCursor => ErrorKind::Validation,
            // A mismatch is a bug in the caller. An overflow can come from real data, but
            // no single input is wrong, so it is not a validation failure either.
            Self::CurrencyMismatch { .. } | Self::AmountOverflow => ErrorKind::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MISMATCH: KernelError = KernelError::CurrencyMismatch {
        left: Currency::COP,
        right: Currency::USD,
    };

    const EVERY_ERROR: [KernelError; 7] = [
        KernelError::InvalidCurrency,
        KernelError::InvalidAmount,
        KernelError::InvalidId,
        KernelError::InvalidPageLimit,
        KernelError::InvalidCursor,
        MISMATCH,
        KernelError::AmountOverflow,
    ];

    /// The table of spec §6.2. The `match` has no wildcard arm, so a new variant does not
    /// compile until it has its row here; list it in `EVERY_ERROR` as well.
    fn expected(error: KernelError) -> (&'static str, ErrorKind) {
        match error {
            KernelError::InvalidCurrency => ("INVALID_CURRENCY", ErrorKind::Validation),
            KernelError::InvalidAmount => ("INVALID_AMOUNT", ErrorKind::Validation),
            KernelError::InvalidId => ("INVALID_ID", ErrorKind::Validation),
            KernelError::InvalidPageLimit => ("INVALID_PAGE_LIMIT", ErrorKind::Validation),
            KernelError::InvalidCursor => ("INVALID_CURSOR", ErrorKind::Validation),
            KernelError::CurrencyMismatch { .. } => ("CURRENCY_MISMATCH", ErrorKind::Internal),
            KernelError::AmountOverflow => ("AMOUNT_OVERFLOW", ErrorKind::Internal),
        }
    }

    #[test]
    fn kernel_errors_keep_their_codes_and_kinds() {
        for error in EVERY_ERROR {
            let (code, kind) = expected(error);
            assert_eq!(error.code(), code);
            assert_eq!(error.kind(), kind, "{code}");
        }
    }

    #[test]
    fn every_kernel_error_has_a_developer_message() {
        for error in EVERY_ERROR {
            assert!(!error.to_string().is_empty(), "{}", error.code());
        }
    }

    #[test]
    fn a_mismatch_does_not_name_the_currencies_in_its_message() {
        let message = MISMATCH.to_string();
        assert!(!message.contains("COP"), "{message}");
        assert!(!message.contains("USD"), "{message}");
    }

    #[test]
    fn an_app_error_is_usable_behind_a_trait_object() {
        let error: Box<dyn AppError> = Box::new(KernelError::InvalidId);
        assert_eq!(error.code(), "INVALID_ID");
        assert_eq!(error.kind(), ErrorKind::Validation);
        let as_std: &dyn std::error::Error = &*error;
        assert!(as_std.source().is_none());
    }
}
