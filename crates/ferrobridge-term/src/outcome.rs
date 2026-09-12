// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! What each operation answers, as one enum per operation.
//!
//! A negative answer the operation states in its own `out` parameters is a
//! variant here; an answer the server refused is a
//! [`crate::error::Error`] carrying the upstream status and body. Nothing is
//! ever flattened into an empty value.

use crate::concept::{Designation, Match, Property};
use crate::error::{Outcome, UpstreamError};

/// What `CodeSystem/$lookup` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum LookupOutcome {
    /// The server knows the code and returned its details.
    Found {
        /// `name`, the code system's name. The operation declares it `1..1`.
        name: String,
        /// `version`, the code system version the answer is from.
        version: Option<String>,
        /// `display`, the preferred display. The operation declares it `1..1`.
        display: String,
        /// Every `designation` the server returned.
        designations: Vec<Designation>,
        /// Every `property` the server returned.
        properties: Vec<Property>,
    },
    /// The server does not know the code, or does not know its system.
    NotFound {
        /// The upstream status and body, with any `tx-issue-type` coding.
        outcome: Box<UpstreamError>,
    },
}

/// What `ConceptMap/$translate` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum TranslateOutcome {
    /// `result` was `true`: the server found at least one match.
    ///
    /// Every match is carried with its equivalence, including the ones that
    /// are not translations, because the specification asks the caller to
    /// check `match.equivalence` for each one.
    Translated {
        /// The matches, in the order the server sent them.
        matches: Vec<Match>,
        /// `message`, when the server sent one.
        message: Option<String>,
    },
    /// `result` was `false`: the server found no acceptable match.
    NoMatch {
        /// `message`, the server's error detail for a false `result`.
        message: Option<String>,
    },
}

impl TranslateOutcome {
    /// Returns every match that is a translation.
    ///
    /// Only `equivalent` and `equal` matches are yielded
    /// (<https://hl7.org/fhir/R4/valueset-concept-map-equivalence.html>), so a
    /// `wider`, `narrower` or `inexact` match never reaches a target system as
    /// a translation.
    pub fn accepted(&self) -> impl Iterator<Item = &Match> {
        let matches = match self {
            Self::Translated { matches, .. } => matches.as_slice(),
            Self::NoMatch { .. } => &[],
        };
        matches.iter().filter(|found| found.accepted())
    }
}

/// What `ValueSet/$validate-code` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum ValidateOutcome {
    /// `result` was `true`: the code is a member of the value set.
    Valid {
        /// `display`, "a valid display for the concept if the system wishes to
        /// display this to a user".
        display: Option<String>,
        /// `message`, which carries hints and warnings beside a true `result`.
        message: Option<String>,
    },
    /// `result` was `false`: the code is not a member of the value set.
    Invalid {
        /// `message`, the server's error detail.
        message: Option<String>,
        /// `issues`, the itemised `OperationOutcome` the terminology ecosystem
        /// asks for beside a false `result`
        /// (<https://hl7.org/fhir/uv/tx-ecosystem/requirements.html>).
        outcome: Option<Outcome>,
    },
}

impl ValidateOutcome {
    /// Returns whether the code is a member of the value set.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }
}

/// One operation to send, for a single call or as a batch entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    /// `CodeSystem/$lookup`.
    Lookup {
        /// The code system the code is in.
        system: String,
        /// The code to look up.
        code: String,
        /// The code system version, when the source data named one.
        version: Option<String>,
    },
    /// `ConceptMap/$translate`.
    Translate {
        /// The code system the code is in.
        system: String,
        /// The code to translate.
        code: String,
        /// The code system to translate into.
        target_system: String,
        /// The canonical URL of the concept map to use, when one is named.
        concept_map_url: Option<String>,
    },
    /// `ValueSet/$validate-code`.
    ValidateCode {
        /// The canonical URL of the value set to validate against.
        value_set_url: String,
        /// The code system the code is in.
        system: String,
        /// The code to validate.
        code: String,
    },
}

impl Request {
    /// Returns the operation, as `Resource/$code`.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        match self {
            Self::Lookup { .. } => crate::LOOKUP,
            Self::Translate { .. } => crate::TRANSLATE,
            Self::ValidateCode { .. } => crate::VALIDATE_CODE,
        }
    }
}

/// What one batch entry answered, in the shape of the request that produced
/// it.
#[derive(Debug)]
#[non_exhaustive]
pub enum BatchOutcome {
    /// The answer to a [`Request::Lookup`].
    Lookup(LookupOutcome),
    /// The answer to a [`Request::Translate`].
    Translate(TranslateOutcome),
    /// The answer to a [`Request::ValidateCode`].
    ValidateCode(ValidateOutcome),
}

#[cfg(test)]
mod tests {
    use super::{Request, TranslateOutcome, ValidateOutcome};
    use crate::concept::{Concept, Equivalence, Match};

    fn found(equivalence: &str, code: &str) -> Match {
        Match {
            equivalence: Some(Equivalence::new(equivalence)),
            concept: Some(Concept {
                system: Some("http://example.org/target".to_owned()),
                code: Some(code.to_owned()),
                ..Concept::default()
            }),
            source: None,
        }
    }

    #[test]
    fn accepted_yields_only_the_equivalent_and_equal_matches() {
        let outcome = TranslateOutcome::Translated {
            matches: vec![
                found("wider", "broad"),
                found("equivalent", "same"),
                found("inexact", "close"),
                found("equal", "identical"),
            ],
            message: None,
        };
        let accepted: Vec<Option<String>> = outcome
            .accepted()
            .map(|found| {
                found
                    .concept
                    .as_ref()
                    .and_then(|concept| concept.code.clone())
            })
            .collect();
        assert_eq!(
            vec![Some("same".to_owned()), Some("identical".to_owned())],
            accepted
        );
    }

    #[test]
    fn a_no_match_outcome_accepts_nothing() {
        let outcome = TranslateOutcome::NoMatch {
            message: Some("no map".to_owned()),
        };
        assert_eq!(0, outcome.accepted().count());
    }

    #[test]
    fn an_invalid_outcome_is_not_valid() {
        let outcome = ValidateOutcome::Invalid {
            message: Some("not in the value set".to_owned()),
            outcome: None,
        };
        assert!(!outcome.is_valid());
    }

    #[test]
    fn a_request_names_its_operation() {
        let lookup = Request::Lookup {
            system: "http://example.org/cs".to_owned(),
            code: "a".to_owned(),
            version: None,
        };
        assert_eq!("CodeSystem/$lookup", lookup.operation());
    }
}
