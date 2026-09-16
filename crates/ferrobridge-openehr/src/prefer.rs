// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `Prefer` request header and the body shape it selects.

use openehr_its::rest::generated::common::Identifier;

/// What the caller wants back from a state-changing call.
///
/// ITS-REST 1.1.0 §Requests and responses/HTTP headers/Prefer defines the
/// three values and tells clients to send one explicitly: "Although the
/// current default behavior is equivalent to `Prefer=minimal`, this might
/// change in the near future to `Prefer=identifier`." The client therefore
/// never omits the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Prefer {
    /// No body: the answer is the status, the `ETag` and the `Location`.
    Minimal,
    /// Only the identifier of the affected resource.
    Identifier,
    /// The full resource representation.
    Representation,
}

impl Prefer {
    /// Returns the header value this preference travels as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "return=minimal",
            Self::Identifier => "return=identifier",
            Self::Representation => "return=representation",
        }
    }
}

impl std::fmt::Display for Prefer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a successful state-changing call returned, per the [`Prefer`] it was
/// asked with.
#[derive(Debug, Clone)]
pub enum Returned<T> {
    /// [`Prefer::Minimal`]: the service sent no representation.
    Minimal,
    /// [`Prefer::Identifier`]: the service sent the resource identifier.
    Identifier(Identifier),
    /// [`Prefer::Representation`]: the service sent the whole resource.
    Representation(Box<T>),
}

#[cfg(test)]
mod tests {
    use super::Prefer;

    #[test]
    fn each_preference_renders_its_rfc_7240_value() {
        assert_eq!("return=minimal", Prefer::Minimal.as_str());
        assert_eq!("return=identifier", Prefer::Identifier.as_str());
        assert_eq!("return=representation", Prefer::Representation.as_str());
    }
}
