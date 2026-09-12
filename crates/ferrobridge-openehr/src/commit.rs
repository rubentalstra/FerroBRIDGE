// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The committal metadata a `POST` or `PUT` of a COMPOSITION carries.
//!
//! ITS-REST 1.1.0 §Requests and responses/HTTP headers/openehr-version and
//! openehr-audit-details defines these headers in prose only: the three
//! `OpenAPI` documents declare none of them. The header names are the 1.1.0
//! ones; the same section's deprecation table maps the 1.0.3 spellings
//! (`openEHR-VERSION`, `openEHR-AUDIT_DETAILS`, `openEHR-TEMPLATE_ID`, which
//! carried the attribute path in the header NAME) onto them, and this client
//! sends only the 1.1.0 form.

use crate::error::Error;
use crate::ids::TemplateId;
use http::HeaderValue;

/// The `openehr-version` header name.
pub(crate) const VERSION_HEADER: &str = "openehr-version";
/// The `openehr-audit-details` header name.
pub(crate) const AUDIT_DETAILS_HEADER: &str = "openehr-audit-details";
/// The `openehr-template-id` header name.
pub(crate) const TEMPLATE_ID_HEADER: &str = "openehr-template-id";

/// A `VERSION.lifecycle_state` code, as `code_string`.
///
/// The code set is the openEHR Version Lifecycle State vocabulary, which
/// ITS-REST 1.1.0 names for this header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LifecycleState(String);

/// An `AUDIT_DETAILS.change_type` code, as `code_string`.
///
/// The code set is the openEHR Audit Change Type vocabulary, which ITS-REST
/// 1.1.0 names for this header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChangeType(String);

/// A code that would not survive the header grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CodeError {
    /// The code was empty.
    #[error("a {kind} code must not be empty")]
    Empty {
        /// The code kind that was being built.
        kind: &'static str,
    },
    /// The code carried a character the quoted header value cannot hold.
    #[error("a {kind} code must not contain {character:?}")]
    ForbiddenCharacter {
        /// The code kind that was being built.
        kind: &'static str,
        /// The offending character.
        character: char,
    },
}

/// Returns `text` as a quotable header value part of `kind`.
fn quotable(kind: &'static str, text: &str) -> Result<String, CodeError> {
    if text.is_empty() {
        return Err(CodeError::Empty { kind });
    }
    if let Some(character) = text.chars().find(|c| *c == '"' || c.is_control()) {
        return Err(CodeError::ForbiddenCharacter { kind, character });
    }
    Ok(text.to_owned())
}

impl LifecycleState {
    /// Returns the lifecycle state `code` names.
    ///
    /// # Errors
    /// Returns [`CodeError`] when `code` is empty or carries a quote or a
    /// control character.
    pub fn new(code: &str) -> Result<Self, CodeError> {
        quotable("lifecycle_state", code).map(Self)
    }

    /// Returns the code as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ChangeType {
    /// Returns the change type `code` names.
    ///
    /// # Errors
    /// Returns [`CodeError`] when `code` is empty or carries a quote or a
    /// control character.
    pub fn new(code: &str) -> Result<Self, CodeError> {
        quotable("change_type", code).map(Self)
    }

    /// Returns the code as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A reference to the committer in an external demographic system,
/// `committer.external_ref` of the `openehr-audit-details` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitterRef {
    /// `committer.external_ref.id`.
    pub id: String,
    /// `committer.external_ref.namespace`.
    pub namespace: String,
    /// `committer.external_ref.type`, the demographic class name.
    pub party_type: String,
}

/// Who is committing, `committer` of the `openehr-audit-details` header.
///
/// This is the header grammar of ITS-REST 1.1.0 §Requests and
/// responses/HTTP headers/openehr-version and openehr-audit-details, which
/// spells a `PARTY_PROXY` as attribute paths rather than as the RM JSON, so it
/// is the header's own model and not a second copy of `PARTY_IDENTIFIED`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committer {
    /// `committer.name`.
    pub name: String,
    /// `committer.external_ref`, when the deployment has one.
    pub external_ref: Option<CommitterRef>,
}

/// The committal metadata one commit carries.
///
/// Every member is optional: "None of these headers are mandatory, but
/// whatever is provided it MUST be merged with the default VERSION and
/// `VERSION.audit_details` attributes on commit runtime" (ITS-REST 1.1.0
/// §Requests and responses/HTTP headers/openehr-version and
/// openehr-audit-details). `time_committed` is always the server's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitContext {
    /// `VERSION.lifecycle_state`, sent as `openehr-version`.
    pub lifecycle_state: Option<LifecycleState>,
    /// `AUDIT_DETAILS.change_type`.
    pub change_type: Option<ChangeType>,
    /// `AUDIT_DETAILS.committer`.
    pub committer: Option<Committer>,
    /// `AUDIT_DETAILS.description.value`.
    pub description: Option<String>,
    /// `AUDIT_DETAILS.system_id`; the server sets its own when this is absent.
    pub system_id: Option<String>,
    /// The template the composition is built from, sent as
    /// `openehr-template-id`.
    pub template_id: Option<TemplateId>,
}

/// One rendered commit header, ready for the request.
#[derive(Debug)]
pub(crate) struct CommitHeader {
    /// The header name.
    pub(crate) name: &'static str,
    /// The header value.
    pub(crate) value: HeaderValue,
}

impl CommitContext {
    /// Returns the headers this context renders into, in wire order.
    ///
    /// # Errors
    /// Returns [`Error::HeaderValue`] when a member carries text no HTTP
    /// header value can hold.
    pub(crate) fn headers(&self) -> Result<Vec<CommitHeader>, Error> {
        let mut rendered: Vec<(&'static str, String)> = Vec::new();
        if let Some(state) = self.lifecycle_state.as_ref() {
            rendered.push((
                VERSION_HEADER,
                format!("lifecycle_state.code_string=\"{}\"", state.as_str()),
            ));
        }
        if let Some(change_type) = self.change_type.as_ref() {
            rendered.push((
                AUDIT_DETAILS_HEADER,
                format!("change_type.code_string=\"{}\"", change_type.as_str()),
            ));
        }
        if let Some(description) = self.description.as_ref() {
            rendered.push((
                AUDIT_DETAILS_HEADER,
                format!("description.value=\"{description}\""),
            ));
        }
        if let Some(committer) = self.committer.as_ref() {
            rendered.push((AUDIT_DETAILS_HEADER, committer_value(committer)));
        }
        if let Some(system_id) = self.system_id.as_ref() {
            rendered.push((AUDIT_DETAILS_HEADER, format!("system_id=\"{system_id}\"")));
        }
        if let Some(template_id) = self.template_id.as_ref() {
            rendered.push((TEMPLATE_ID_HEADER, template_id.as_str().to_owned()));
        }
        rendered
            .into_iter()
            .map(|(name, value)| {
                HeaderValue::from_str(&value)
                    .map(|value| CommitHeader { name, value })
                    .map_err(|source| Error::HeaderValue {
                        header: name,
                        source,
                    })
            })
            .collect()
    }
}

/// Returns the `committer` attribute list of an `openehr-audit-details` value.
fn committer_value(committer: &Committer) -> String {
    let name = &committer.name;
    match committer.external_ref.as_ref() {
        None => format!("committer.name=\"{name}\""),
        Some(reference) => format!(
            "committer.name=\"{name}\",committer.external_ref.id=\"{}\",committer.external_ref.namespace=\"{}\",committer.external_ref.type=\"{}\"",
            reference.id, reference.namespace, reference.party_type
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AUDIT_DETAILS_HEADER, ChangeType, CommitContext, Committer, CommitterRef, LifecycleState,
        TEMPLATE_ID_HEADER, VERSION_HEADER,
    };
    use crate::error::Error;
    use crate::ids::TemplateId;

    fn rendered(context: &CommitContext) -> Vec<(&'static str, String)> {
        context
            .headers()
            .expect("the context renders")
            .into_iter()
            .map(|header| {
                (
                    header.name,
                    header
                        .value
                        .to_str()
                        .expect("a rendered value is ASCII")
                        .to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn the_headers_carry_the_1_1_0_value_form() {
        let context = CommitContext {
            lifecycle_state: Some(LifecycleState::new("532").expect("a legal code")),
            change_type: Some(ChangeType::new("251").expect("a legal code")),
            committer: Some(Committer {
                name: "John Doe".to_owned(),
                external_ref: Some(CommitterRef {
                    id: "BC8132EA".to_owned(),
                    namespace: "demographic".to_owned(),
                    party_type: "PERSON".to_owned(),
                }),
            }),
            description: Some("An updated composition".to_owned()),
            system_id: Some("example.openehr.systemid".to_owned()),
            template_id: Some(TemplateId::new("Vital Signs").expect("a legal template id")),
        };
        assert_eq!(
            vec![
                (VERSION_HEADER, "lifecycle_state.code_string=\"532\"".to_owned()),
                (AUDIT_DETAILS_HEADER, "change_type.code_string=\"251\"".to_owned()),
                (AUDIT_DETAILS_HEADER, "description.value=\"An updated composition\"".to_owned()),
                (
                    AUDIT_DETAILS_HEADER,
                    "committer.name=\"John Doe\",committer.external_ref.id=\"BC8132EA\",committer.external_ref.namespace=\"demographic\",committer.external_ref.type=\"PERSON\"".to_owned()
                ),
                (AUDIT_DETAILS_HEADER, "system_id=\"example.openehr.systemid\"".to_owned()),
                (TEMPLATE_ID_HEADER, "Vital Signs".to_owned()),
            ],
            rendered(&context)
        );
    }

    #[test]
    fn an_empty_context_renders_no_header() {
        assert!(rendered(&CommitContext::default()).is_empty());
    }

    #[test]
    fn a_code_refuses_a_quote() {
        assert!(ChangeType::new("2\"51").is_err());
        assert!(LifecycleState::new("").is_err());
    }

    #[test]
    fn a_description_that_is_not_a_header_value_is_a_typed_error() {
        let context = CommitContext {
            description: Some("first\nsecond".to_owned()),
            ..CommitContext::default()
        };
        let error = context.headers().expect_err("a newline is refused");
        assert!(
            matches!(error, Error::HeaderValue { header, .. } if header == AUDIT_DETAILS_HEADER)
        );
    }
}
