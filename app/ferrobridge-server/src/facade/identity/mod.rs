// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identity layer: the typed identifiers, the derivation and the store.
//!
//! `docs/architecture.md` §9 fixes the shape. A FHIR resource id derives from
//! the entry's `LOCATABLE.uid` when the entry carries one and otherwise from a
//! digest over the version container, the entry path and the split occurrence;
//! whichever input produced it, the store records it and the store wins from
//! then on, because "once assigned, this value never changes"
//! (<https://hl7.org/fhir/R4/resource.html>).
//!
//! The identifiers the openEHR side owns are the ones the CDR module
//! already validates (`crate::cdr::ids`), so nothing here restates
//! them. [`FhirResourceId`], [`ExternalResourceId`] and [`PersonId`] are the
//! three this side owns.

pub mod claims;
pub mod derive;
pub mod record;
pub mod redb_store;
pub mod store;

use core::fmt;

/// Why a value cannot be one of this module's identifiers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdError {
    /// The text carries no characters.
    #[error("a {kind} cannot be empty")]
    Empty {
        /// Which identifier was being built.
        kind: &'static str,
    },
    /// The text is longer than the grammar admits.
    #[error("a {kind} is at most {limit} characters, and this one is {found}")]
    TooLong {
        /// Which identifier was being built.
        kind: &'static str,
        /// The ceiling the grammar sets.
        limit: usize,
        /// How long the text was.
        found: usize,
    },
    /// The text carries a character the grammar does not admit.
    #[error("a {kind} cannot carry {character:?}")]
    ForbiddenCharacter {
        /// Which identifier was being built.
        kind: &'static str,
        /// The first character that is not admitted.
        character: char,
    },
}

/// The longest FHIR `id`.
///
/// `Resource.id` is `[A-Za-z0-9\-\.]{1,64}`
/// (<https://hl7.org/fhir/R4/resource.html>).
const ID_LIMIT: usize = 64;

/// Whether `text` matches the R4 `Resource.id` grammar.
///
/// The grammar is `[A-Za-z0-9\-\.]{1,64}`
/// (<https://hl7.org/fhir/R4/resource.html>).
#[must_use]
pub fn is_fhir_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= ID_LIMIT
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// The logical id of a FHIR resource this bridge serves.
///
/// The value always matches the R4 `Resource.id` grammar, so a resource built
/// with one cannot carry an id a FHIR client would refuse.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FhirResourceId(String);

impl FhirResourceId {
    /// Returns the resource id `text` names.
    ///
    /// # Errors
    ///
    /// Returns [`IdError`] when `text` is empty, longer than 64 characters, or
    /// carries a character outside `[A-Za-z0-9\-\.]`.
    pub fn new(text: &str) -> Result<Self, IdError> {
        if text.is_empty() {
            return Err(IdError::Empty { kind: "FHIR id" });
        }
        if text.len() > ID_LIMIT {
            return Err(IdError::TooLong {
                kind: "FHIR id",
                limit: ID_LIMIT,
                found: text.len(),
            });
        }
        if let Some(character) = text
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '.'))
        {
            return Err(IdError::ForbiddenCharacter {
                kind: "FHIR id",
                character,
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns the id as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FhirResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The id a sending system gave a resource, as it arrived.
///
/// A sending system's id is opaque to this bridge: it is a key into the
/// identity map and never a value the bridge writes into a document, so it is
/// kept as sent and is not held to the R4 `id` grammar. The type is distinct
/// from [`FhirResourceId`] so the two cannot be swapped at a call site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalResourceId(String);

impl ExternalResourceId {
    /// Returns the external id `text` names.
    ///
    /// # Errors
    ///
    /// Returns [`IdError`] when `text` is empty or carries a control
    /// character, which no header, URL or key can hold.
    pub fn new(text: &str) -> Result<Self, IdError> {
        if text.is_empty() {
            return Err(IdError::Empty {
                kind: "external resource id",
            });
        }
        if let Some(character) = text.chars().find(|c| c.is_control()) {
            return Err(IdError::ForbiddenCharacter {
                kind: "external resource id",
                character,
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns the external identity of one derived resource id.
    ///
    /// A derived id carries only base32 characters, so it is always a legal
    /// external identity; this is the total conversion the identity-map key
    /// takes (`docs/architecture.md` §9).
    #[must_use]
    pub fn of_digest(digest: &FhirResourceId) -> Self {
        Self(digest.as_str().to_owned())
    }

    /// Returns the id as the sending system wrote it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExternalResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The subject one EHR belongs to, as the deployment identifies a person.
///
/// The value is the `subject_namespace` and `subject_id` pair the ITS-REST
/// EHR lookup takes (`ehr-codegen.openapi.yaml`, `ehr_get_by_subject`), so a
/// key here resolves to exactly one CDR query.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PersonId {
    /// The namespace the identifier belongs to.
    namespace: String,
    /// The identifier inside that namespace.
    id: String,
}

impl PersonId {
    /// Returns the person `id` in `namespace` names.
    ///
    /// # Errors
    ///
    /// Returns [`IdError`] when either half is empty or carries a control
    /// character.
    pub fn new(namespace: &str, id: &str) -> Result<Self, IdError> {
        for (kind, text) in [("subject namespace", namespace), ("subject id", id)] {
            if text.is_empty() {
                return Err(IdError::Empty { kind });
            }
            if let Some(character) = text.chars().find(|c| c.is_control()) {
                return Err(IdError::ForbiddenCharacter { kind, character });
            }
        }
        Ok(Self {
            namespace: namespace.to_owned(),
            id: id.to_owned(),
        })
    }

    /// Returns the namespace the identifier belongs to.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the identifier inside its namespace.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for PersonId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}|{}", self.namespace, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::{ExternalResourceId, FhirResourceId, IdError, PersonId, is_fhir_id};

    #[test]
    fn a_fhir_id_follows_the_r4_grammar() {
        assert!(FhirResourceId::new("abc-123.4").is_ok());
        assert!(matches!(
            FhirResourceId::new(""),
            Err(IdError::Empty { .. })
        ));
        assert!(matches!(
            FhirResourceId::new("a_b"),
            Err(IdError::ForbiddenCharacter { character: '_', .. })
        ));
        assert!(matches!(
            FhirResourceId::new(&"a".repeat(65)),
            Err(IdError::TooLong { limit: 64, .. })
        ));
    }

    #[test]
    fn an_object_version_id_is_no_fhir_id() {
        assert!(
            !is_fhir_id("8849182c-82ad-4088-a07f-48ead4180515::ferroehr::1"),
            "the colons of an OBJECT_VERSION_ID are outside the R4 id grammar"
        );
        assert!(is_fhir_id("8849182c-82ad-4088-a07f-48ead4180515"));
    }

    #[test]
    fn an_external_id_keeps_what_the_sender_wrote() {
        let id = ExternalResourceId::new("urn:uuid:9f0c6b16-0c9e-4d10-9a6c-cd3f3f9b3a41")
            .expect("a URN is a legal external id");
        assert_eq!("urn:uuid:9f0c6b16-0c9e-4d10-9a6c-cd3f3f9b3a41", id.as_str());
        assert!(matches!(
            ExternalResourceId::new("a\nb"),
            Err(IdError::ForbiddenCharacter { .. })
        ));
    }

    #[test]
    fn a_person_id_carries_both_halves_of_the_cdr_lookup() {
        let person = PersonId::new("http://example.org/fhir/sid/mrn", "p-1")
            .expect("both halves are non-empty");
        assert_eq!("http://example.org/fhir/sid/mrn", person.namespace());
        assert_eq!("p-1", person.id());
        assert!(matches!(
            PersonId::new("", "p-1"),
            Err(IdError::Empty { .. })
        ));
    }
}
