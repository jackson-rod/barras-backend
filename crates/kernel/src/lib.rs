//! Domain model and ports shared by every layer, with no I/O.

mod clock;
mod currency;
mod entitlements;
mod error;
mod id;
mod money;
mod page;

#[cfg(any(test, feature = "testing"))]
pub use clock::FixedClock;
pub use clock::{Clock, SystemClock};
pub use currency::Currency;
pub use entitlements::{Entitlements, Feature, Plan};
pub use error::{AppError, ErrorKind, KernelError};
pub use id::UserId;
pub use money::Money;
pub use page::{Cursor, Page, PageLimit, PageRequest};

/// What `define_id!` expands to in another crate. Not part of the public API.
#[doc(hidden)]
pub mod __private {
    pub use uuid::Uuid;

    pub use crate::id::parse_canonical;
}
