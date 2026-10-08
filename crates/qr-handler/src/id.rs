//! Handler identifiers.

use std::{borrow::Cow, fmt};

/// A handler's name in a URL, and the value stored in `bindings.handler_id`.
///
/// It is part of the mount path and of every row a handler owns, so it is a
/// compatibility promise the moment a code is bound to it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandlerId(Cow<'static, str>);

#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not a handler id: expected [a-z0-9][a-z0-9-]{{0,31}}")]
pub struct InvalidHandlerId(String);

impl HandlerId {
    /// For a handler naming itself. The shape is checked in tests rather than at
    /// runtime, because a literal that is wrong is a bug in this repository.
    pub const fn from_static(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    pub fn parse(id: &str) -> Result<Self, InvalidHandlerId> {
        if is_valid(id) {
            Ok(Self(Cow::Owned(id.to_owned())))
        } else {
            Err(InvalidHandlerId(id.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `[a-z0-9][a-z0-9-]{0,31}`, which is also what the schema's CHECK enforces.
pub fn is_valid(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    id.len() <= 32 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl fmt::Display for HandlerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_a_url_and_the_schema_both_accept() {
        for good in ["photo-album", "forward", "a", "m3trics-2"] {
            assert!(HandlerId::parse(good).is_ok(), "{good} should parse");
        }
        for bad in [
            "",
            "-leading",
            "Photo",
            "with_underscore",
            "with space",
            &"a".repeat(33),
        ] {
            assert!(HandlerId::parse(bad).is_err(), "{bad} should not parse");
        }
    }
}
