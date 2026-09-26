// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `spec.openEhrConfig.archetype` value.

use core::fmt;
use core::str::FromStr;

/// Why a `spec.openEhrConfig.archetype` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArchetypeIdError {
    /// The value is not `<qualified_rm_entity>.<domain_concept>.<version_id>`.
    #[error("`{value}` is not `rm_originator-rm_name-rm_entity.concept.vN`")]
    Shape {
        /// The refused value.
        value: String,
    },
    /// The first part is not `rm_originator-rm_name-rm_entity`.
    #[error("`{value}` is not `rm_originator-rm_name-rm_entity`")]
    QualifiedRmEntity {
        /// The refused first part.
        value: String,
    },
    /// The second part is not a concept name with optional specialisations.
    #[error("`{value}` is not `concept_name{{-specialisation}}`")]
    DomainConcept {
        /// The refused second part.
        value: String,
    },
    /// The third part is not `v` followed by a version number.
    #[error("`{value}` is not `v0` or `v` followed by a non-zero-leading number")]
    VersionId {
        /// The refused third part.
        value: String,
    },
}

/// An openEHR archetype id.
///
/// The lexical form is `rm_originator '-' rm_name '-' rm_entity '.'
/// concept_name {'-' specialisation}* '.v' number`, and the syntax appendix
/// gives `version-id = 'v', ('0' | non-zero-digit, [number])` and
/// `alphanum-str = letter, {letter | digit | '_'}` (openEHR BASE Release
/// 1.2.0, §5.4.10 `ARCHETYPE_ID` Class and the Base Types syntax appendix,
/// <https://specifications.openehr.org/releases/BASE/Release-1.2.0/base_types.html>).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchetypeId {
    value: String,
    version: u32,
}

impl ArchetypeId {
    /// Returns the archetype id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns the number of the `version_id` axis.
    #[must_use]
    pub const fn version_number(&self) -> u32 {
        self.version
    }
}

impl fmt::Display for ArchetypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl FromStr for ArchetypeId {
    type Err = ArchetypeIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut parts = value.split('.');
        let (Some(qualified_rm_entity), Some(domain_concept), Some(version_id), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ArchetypeIdError::Shape {
                value: value.to_owned(),
            });
        };

        let axes: Vec<&str> = qualified_rm_entity.split('-').collect();
        if axes.len() != 3 || !axes.iter().all(|axis| is_alphanum_str(axis)) {
            return Err(ArchetypeIdError::QualifiedRmEntity {
                value: qualified_rm_entity.to_owned(),
            });
        }

        if !domain_concept.split('-').all(is_alphanum_str) {
            return Err(ArchetypeIdError::DomainConcept {
                value: domain_concept.to_owned(),
            });
        }

        let version = parse_version_id(version_id).ok_or_else(|| ArchetypeIdError::VersionId {
            value: version_id.to_owned(),
        })?;

        Ok(Self {
            value: value.to_owned(),
            version,
        })
    }
}

/// Whether a string is `letter, {letter | digit | '_'}`.
fn is_alphanum_str(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && characters.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parses `version-id = 'v', ('0' | non-zero-digit, [number])`.
fn parse_version_id(version_id: &str) -> Option<u32> {
    let digits = version_id.strip_prefix('v')?;
    if digits == "0" {
        return Some(0);
    }
    let mut characters = digits.chars();
    let first = characters.next()?;
    if !first.is_ascii_digit() || first == '0' {
        return None;
    }
    if !characters.all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}
