//! Cursor pagination: how a list is asked for and returned one page at a time.

use crate::error::KernelError;

const MAX_CURSOR_LENGTH: usize = 512;

/// How many items one page may hold: from 1 to 50.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageLimit(u32);

impl PageLimit {
    pub const MAX: u32 = 50;
    /// The limit of a request that names none.
    pub const DEFAULT: Self = Self(20);

    pub fn new(value: u32) -> Result<Self, KernelError> {
        if (1..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(KernelError::InvalidPageLimit)
        }
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

/// An opaque token that marks where the next page starts: 1 to 512 characters of
/// `A-Z a-z 0-9 - _`. What it encodes is the business of the repository that issued it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cursor(String);

impl Cursor {
    pub fn new(token: impl Into<String>) -> Result<Self, KernelError> {
        let token = token.into();
        let has_valid_length = (1..=MAX_CURSOR_LENGTH).contains(&token.len());
        let is_in_the_set = token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        if has_valid_length && is_in_the_set {
            Ok(Self(token))
        } else {
            Err(KernelError::InvalidCursor)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a list use case is asked for: how many items, and after which cursor. No cursor
/// means the first page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub limit: PageLimit,
    pub after: Option<Cursor>,
}

/// One page of a list. `next` is the cursor of the following page, and `None` on the last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<Cursor>,
}

impl<T> Page<T> {
    /// The same page with every item converted, for the step from a domain type to a DTO.
    pub fn map<U>(self, convert: impl FnMut(T) -> U) -> Page<U> {
        Page {
            items: self.items.into_iter().map(convert).collect(),
            next: self.next,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppError;

    fn cursor(token: &str) -> Cursor {
        Cursor::new(token).unwrap()
    }

    #[test]
    fn a_page_limit_is_from_1_to_50() {
        for value in [1, 20, 50] {
            assert_eq!(PageLimit::new(value).unwrap().get(), value);
        }
        for value in [0, 51, u32::MAX] {
            let error = PageLimit::new(value).unwrap_err();
            assert_eq!(error.code(), "INVALID_PAGE_LIMIT", "{value}");
        }
    }

    #[test]
    fn the_default_limit_is_20_and_the_largest_is_50() {
        assert_eq!(PageLimit::DEFAULT.get(), 20);
        assert_eq!(PageLimit::MAX, 50);
        assert_eq!(PageLimit::new(PageLimit::MAX).unwrap().get(), 50);
    }

    #[test]
    fn a_cursor_is_1_to_512_characters_of_its_set() {
        let longest = "a".repeat(512);
        for token in [
            "a",
            "Z",
            "0",
            "-",
            "_",
            "AZaz09-_",
            "eyJpZCI6IjAxOTNhNWI4In0",
            longest.as_str(),
        ] {
            assert_eq!(cursor(token).as_str(), token);
        }
    }

    #[test]
    fn any_other_cursor_is_refused() {
        let too_long = "a".repeat(513);
        let too_long_in_bytes_only = "é".repeat(256);
        for token in [
            "",
            too_long.as_str(),
            too_long_in_bytes_only.as_str(),
            "a b",
            "a+b",
            "a/b",
            "a=b",
            "a.b",
            "a\n",
            "ñ",
        ] {
            let error = Cursor::new(token).unwrap_err();
            assert_eq!(error.code(), "INVALID_CURSOR", "{token:?}");
        }
    }

    #[test]
    fn a_cursor_takes_an_owned_token_too() {
        let token = String::from("abc123");
        assert_eq!(Cursor::new(token).unwrap().as_str(), "abc123");
    }

    #[test]
    fn the_refusals_do_not_repeat_the_input() {
        let limit = PageLimit::new(4321).unwrap_err().to_string();
        assert!(!limit.contains("4321"), "{limit}");
        let token = Cursor::new("bad cursor!").unwrap_err().to_string();
        assert!(!token.contains("bad cursor!"), "{token}");
    }

    #[test]
    fn a_page_request_says_how_many_items_and_after_which_cursor() {
        let first = PageRequest {
            limit: PageLimit::DEFAULT,
            after: None,
        };
        let following = PageRequest {
            limit: first.limit,
            after: Some(cursor("abc")),
        };
        assert_eq!(first.after, None);
        assert_eq!(following.limit.get(), 20);
        assert_eq!(following.after, Some(cursor("abc")));
    }

    #[test]
    fn map_changes_the_items_and_keeps_the_cursor() {
        let page = Page {
            items: vec![1, 2, 3],
            next: Some(cursor("next")),
        };
        let mapped = page.map(|number| number.to_string());
        assert_eq!(mapped.items, ["1", "2", "3"]);
        assert_eq!(mapped.next, Some(cursor("next")));
    }

    #[test]
    fn map_keeps_a_last_page_a_last_page() {
        let last: Page<u32> = Page {
            items: Vec::new(),
            next: None,
        };
        let mapped = last.map(u64::from);
        assert!(mapped.items.is_empty());
        assert_eq!(mapped.next, None);
    }
}
