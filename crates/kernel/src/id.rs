//! Typed ids: a UUID that knows what it identifies.

use uuid::Uuid;

use crate::error::KernelError;

/// Defines a typed id: a newtype over a UUID, so that the id of one thing cannot be passed
/// where the id of another is expected.
///
/// ```
/// barras_kernel::define_id!(
///     /// Identifies a trade.
///     TradeId
/// );
///
/// let id = TradeId::generate();
/// assert_eq!(TradeId::parse(&id.to_string()), Ok(id));
/// ```
#[macro_export]
macro_rules! define_id {
    ($(#[$attribute:meta])* $name:ident) => {
        $(#[$attribute])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($crate::__private::Uuid);

        impl $name {
            /// A new id: a UUID version 7, so ids sort by the time they were made.
            pub fn generate() -> Self {
                Self($crate::__private::Uuid::now_v7())
            }

            pub const fn from_uuid(uuid: $crate::__private::Uuid) -> Self {
                Self(uuid)
            }

            pub const fn as_uuid(&self) -> $crate::__private::Uuid {
                self.0
            }

            /// Reads the form `Display` writes and no other: 36 characters, hyphenated,
            /// lower case. Any UUID version is an id.
            pub fn parse(text: &str) -> ::core::result::Result<Self, $crate::KernelError> {
                $crate::__private::parse_canonical(text).map(Self)
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

define_id!(
    /// Identifies a user. Every module refers to it, so it lives in the kernel.
    UserId
);

/// The parser behind every id's `parse`. It is public only so that `define_id!` can reach
/// it from another crate.
pub fn parse_canonical(text: &str) -> Result<Uuid, KernelError> {
    Uuid::try_parse(text)
        .ok()
        // The UUID parser also reads the simple, braced, URN and upper-case forms. An id
        // has one spelling, so only the text that encodes back to itself is accepted.
        .filter(|uuid| {
            let mut buffer = Uuid::encode_buffer();
            *uuid.as_hyphenated().encode_lower(&mut buffer) == *text
        })
        .ok_or(KernelError::InvalidId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppError;

    define_id!(
        /// A second id, to prove that two ids are two types.
        OrderId
    );

    const CANONICAL: &str = "0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1b";

    #[test]
    fn a_generated_id_is_a_uuid_version_7() {
        let id = UserId::generate();
        assert_eq!(id.as_uuid().get_version_num(), 7);
        assert_ne!(id, UserId::generate());
    }

    #[test]
    fn an_id_wraps_the_uuid_it_was_given() {
        let uuid = Uuid::from_u128(0x0193_a5b8_7c2e_7d4f_8a1b_3c5d_7e9f_0a1b);
        assert_eq!(UserId::from_uuid(uuid).as_uuid(), uuid);
    }

    #[test]
    fn parse_reads_the_canonical_form_of_any_version() {
        for text in [
            CANONICAL,
            "00000000-0000-0000-0000-000000000000",
            "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
        ] {
            let id = UserId::parse(text).unwrap();
            assert_eq!(id.to_string(), text);
        }
    }

    #[test]
    fn parse_refuses_every_other_form() {
        for text in [
            "",
            "not-a-uuid",
            "0193A5B8-7C2E-7D4F-8A1B-3C5D7E9F0A1B",
            "0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1B",
            "0193a5b87c2e7d4f8a1b3c5d7e9f0a1b",
            "{0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1b}",
            "urn:uuid:0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1b",
            " 0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1b",
            "0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1b ",
            "0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1",
            "0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1bc",
            "0193a5b8-7c2e-7d4f-8a1b-3c5d7e9f0a1g",
            "0193a5b8_7c2e_7d4f_8a1b_3c5d7e9f0a1b",
            "0193a5b87-c2e-7d4f-8a1b-3c5d7e9f0a1b",
        ] {
            let error = UserId::parse(text).unwrap_err();
            assert_eq!(error.code(), "INVALID_ID", "{text:?}");
        }
    }

    #[test]
    fn display_round_trips_through_parse() {
        let id = UserId::generate();
        let text = id.to_string();
        assert_eq!(text.len(), 36);
        assert_eq!(UserId::parse(&text).unwrap(), id);
    }

    #[test]
    fn the_refusal_does_not_repeat_the_input() {
        let message = UserId::parse("zz-not-an-id-zz").unwrap_err().to_string();
        assert!(!message.contains("zz-not-an-id-zz"), "{message}");
    }

    #[test]
    fn the_macro_defines_an_id_of_its_own_type() {
        let order = OrderId::parse(CANONICAL).unwrap();
        let user = UserId::parse(CANONICAL).unwrap();
        assert_eq!(order, OrderId::from_uuid(user.as_uuid()));
        assert_eq!(order.to_string(), CANONICAL);
        assert_eq!(OrderId::generate().as_uuid().get_version_num(), 7);
    }
}
