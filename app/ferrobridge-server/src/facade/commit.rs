// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The committal metadata every write this facade makes carries.
//!
//! ITS-REST 1.1.0 leaves the committal metadata headers optional ("None of
//! these headers are mandatory"), and the CONTRIBUTION body carries the same
//! facts as schema members instead (`ehr-codegen.openapi.yaml`,
//! `contribution_create`). This module builds both from one description, so a
//! single write and a transaction record the same committer and change type.

use ferrobridge_openehr::commit::ChangeType;
use ferrobridge_openehr::commit::CodeError;
use ferrobridge_openehr::commit::CommitContext;
use ferrobridge_openehr::commit::Committer;
use ferrobridge_openehr::commit::LifecycleState;
use ferrobridge_openehr::ids::IdError;
use ferrobridge_openehr::ids::TemplateId;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_its::rest::generated::common::UpdateAudit;
use openehr_its::rest::generated::common::UpdateAuditData;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentifiedData;
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// The terminology the openEHR audit code sets belong to.
///
/// `AUDIT_DETAILS.change_type` and `VERSION.lifecycle_state` are coded from
/// the openEHR terminology
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html>).
const OPENEHR_TERMINOLOGY: &str = "openehr";

/// The `change_type` code of a first commit, "creation".
const CREATION_CODE: &str = "249";

/// The `change_type` code of a later commit, "modification".
const MODIFICATION_CODE: &str = "251";

/// The `lifecycle_state` code of a committed version, "complete".
const COMPLETE_CODE: &str = "532";

/// Whether a write adds a version container or a version to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The first version of a composition.
    Creation,
    /// A later version of a composition.
    Modification,
}

impl Change {
    /// Returns the openEHR `change_type` code of this change.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Creation => CREATION_CODE,
            Self::Modification => MODIFICATION_CODE,
        }
    }

    /// Returns the openEHR `change_type` rubric of this change.
    #[must_use]
    pub const fn rubric(self) -> &'static str {
        match self {
            Self::Creation => "creation",
            Self::Modification => "modification",
        }
    }
}

/// Why the committal metadata could not be built.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CommitError {
    /// A code the openEHR terminology fixes was refused.
    #[error("an openEHR audit code was refused")]
    Code {
        /// What the code's constructor reported.
        #[source]
        source: Box<CodeError>,
    },
    /// The template identifier cannot travel in the commit header.
    #[error("{template} cannot travel as a template identifier")]
    Template {
        /// The identifier the composition names.
        template: String,
        /// What the identifier's constructor reported.
        #[source]
        source: Box<IdError>,
    },
}

/// Returns the commit headers one composition write carries.
///
/// # Errors
///
/// Returns [`CommitError::Code`] when an openEHR audit code is refused and
/// [`CommitError::Template`] when the template identifier cannot travel.
pub fn context(
    change: Change,
    system_id: &str,
    template: &str,
) -> Result<CommitContext, CommitError> {
    Ok(CommitContext {
        lifecycle_state: Some(LifecycleState::new(COMPLETE_CODE).map_err(refused)?),
        change_type: Some(ChangeType::new(change.code()).map_err(refused)?),
        committer: Some(Committer {
            name: String::from(system_id),
            external_ref: None,
        }),
        description: None,
        system_id: Some(String::from(system_id)),
        template_id: Some(
            TemplateId::new(template).map_err(|source| CommitError::Template {
                template: String::from(template),
                source: Box::new(source),
            })?,
        ),
    })
}

/// Returns the `UpdateAudit` a contribution entry carries.
///
/// The CONTRIBUTION body states the committer and the change type as schema
/// members rather than as headers, so a transaction records exactly what a
/// single write does.
#[must_use]
pub fn audit(change: Change, system_id: &str) -> UpdateAudit {
    UpdateAudit::UpdateAudit(UpdateAuditData {
        _type: Some(String::from("UPDATE_AUDIT")),
        system_id: Some(String::from(system_id)),
        change_type: coded(change.code(), change.rubric()),
        description: None,
        committer: PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(
            PartyIdentifiedData {
                external_ref: None,
                name: Some(String::from(system_id)),
                identifiers: None,
            },
        )),
    })
}

/// Returns the `lifecycle_state` a contribution entry carries.
#[must_use]
pub fn lifecycle() -> DvCodedText {
    coded(COMPLETE_CODE, "complete")
}

/// Wraps a refused openEHR audit code as a commit refusal that carries it.
fn refused(source: CodeError) -> CommitError {
    CommitError::Code {
        source: Box::new(source),
    }
}

/// Returns a `DV_CODED_TEXT` in the openEHR terminology.
fn coded(code: &str, rubric: &str) -> DvCodedText {
    DvCodedText {
        value: String::from(rubric),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: CodePhrase {
            terminology_id: TerminologyId {
                value: String::from(OPENEHR_TERMINOLOGY),
            },
            code_string: String::from(code),
            preferred_term: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, audit, context, lifecycle};
    use openehr_its::rest::generated::common::UpdateAudit;

    #[test]
    fn the_change_codes_are_the_openehr_terminology_ones() {
        assert_eq!("249", Change::Creation.code());
        assert_eq!("251", Change::Modification.code());
        assert_eq!("creation", Change::Creation.rubric());
    }

    #[test]
    fn a_commit_context_carries_the_change_type_the_system_and_the_template() {
        let built = context(Change::Creation, "FerroBRIDGE", "ferrobridge.diagnose.v1")
            .expect("the context builds");
        assert_eq!(
            Some("249"),
            built
                .change_type
                .as_ref()
                .map(ferrobridge_openehr::commit::ChangeType::as_str)
        );
        assert_eq!(Some("FerroBRIDGE"), built.system_id.as_deref());
        assert_eq!(
            Some("ferrobridge.diagnose.v1"),
            built
                .template_id
                .as_ref()
                .map(ferrobridge_openehr::ids::TemplateId::as_str)
        );
    }

    #[test]
    fn the_contribution_audit_states_the_same_change_type_as_the_header() {
        let UpdateAudit::UpdateAudit(data) = audit(Change::Modification, "FerroBRIDGE") else {
            panic!("the facade writes an UPDATE_AUDIT, never an attestation");
        };
        assert_eq!("251", data.change_type.defining_code.code_string);
        assert_eq!(
            "openehr",
            data.change_type.defining_code.terminology_id.value
        );
        assert_eq!("532", lifecycle().defining_code.code_string);
    }

    #[test]
    fn a_template_identifier_a_header_cannot_hold_is_refused() {
        assert!(context(Change::Creation, "FerroBRIDGE", "").is_err());
    }
}
