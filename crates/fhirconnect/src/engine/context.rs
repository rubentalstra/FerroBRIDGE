// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The values a caller passes on the REST call, as one run reads them.
//!
//! `$context` "holds values passed in on the REST call ... only for values
//! that are not in the composition. A context value is referenced from a
//! `manual` `value`"
//! (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`), so
//! this is the one input a run takes that neither document carries. The
//! members are the four the draft REST API chapter declares under the
//! `context` parameter
//! (`docs/specs/fhirconnect/draft-rest-api/rest/input/fsh/operations/ToFhir.fsh`).
//!
//! A member the caller did not supply is a typed refusal
//! ([`crate::engine::traverse::EngineError::ContextMember`]) rather than an
//! invented value.

use fhir_types::r4::reference::Reference;

/// The `ehr_id` member, as the draft spells it.
pub const EHR_ID: &str = "ehr_id";

/// The `patient` member, as the draft spells it.
pub const PATIENT: &str = "patient";

/// The `who` member, as the draft spells it.
pub const WHO: &str = "who";

/// The `onBehalfOf` member, as the draft spells it.
pub const ON_BEHALF_OF: &str = "onBehalfOf";

/// The per-call values one run resolves `$context` against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallContext {
    ehr_id: Option<String>,
    patient: Option<Reference>,
    who: Option<Reference>,
    on_behalf_of: Option<Reference>,
}

impl CallContext {
    /// Creates a context carrying no member.
    ///
    /// A run over it refuses every `$context` path, which is what a call that
    /// supplies no context asks for.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ehr_id: None,
            patient: None,
            who: None,
            on_behalf_of: None,
        }
    }

    /// Returns this context with `id` as the `ehr_id` member.
    #[must_use]
    pub fn with_ehr_id(mut self, id: impl Into<String>) -> Self {
        self.ehr_id = Some(id.into());
        self
    }

    /// Returns this context with `patient` as the `patient` member.
    #[must_use]
    pub fn with_patient(mut self, patient: Reference) -> Self {
        self.patient = Some(patient);
        self
    }

    /// Returns this context with `who` as the `who` member.
    #[must_use]
    pub fn with_who(mut self, who: Reference) -> Self {
        self.who = Some(who);
        self
    }

    /// Returns this context with `organization` as the `onBehalfOf` member.
    #[must_use]
    pub fn with_on_behalf_of(mut self, organization: Reference) -> Self {
        self.on_behalf_of = Some(organization);
        self
    }

    /// Returns the `ehr_id` member.
    #[must_use]
    pub fn ehr_id(&self) -> Option<&str> {
        self.ehr_id.as_deref()
    }

    /// Returns the `patient` member.
    #[must_use]
    pub const fn patient(&self) -> Option<&Reference> {
        self.patient.as_ref()
    }

    /// Returns the `who` member.
    #[must_use]
    pub const fn who(&self) -> Option<&Reference> {
        self.who.as_ref()
    }

    /// Returns the `onBehalfOf` member.
    #[must_use]
    pub const fn on_behalf_of(&self) -> Option<&Reference> {
        self.on_behalf_of.as_ref()
    }

    /// Returns the text a `manual` path writes for the member `name`.
    ///
    /// A reference member answers its literal reference, the form a
    /// `Reference` carries a resource in
    /// (<https://hl7.org/fhir/R4/references.html#literal>); the draft names
    /// no other rendering, so the choice is FerroBRIDGE's own.
    #[must_use]
    pub fn member(&self, name: &str) -> Option<&str> {
        match name {
            EHR_ID => self.ehr_id(),
            PATIENT => literal(self.patient.as_ref()),
            WHO => literal(self.who.as_ref()),
            ON_BEHALF_OF => literal(self.on_behalf_of.as_ref()),
            _ => None,
        }
    }
}

/// Returns the literal reference a `Reference` carries.
fn literal(reference: Option<&Reference>) -> Option<&str> {
    reference?.reference.as_ref()?.value.as_deref()
}

#[cfg(test)]
mod tests {
    use super::CallContext;
    use fhir_types::r4::reference::Reference;

    /// Returns a `Reference` whose literal reference is `text`.
    fn reference(text: &str) -> Reference {
        Reference {
            reference: Some(fhir_types::r4::primitives::String::from(text)),
            ..Reference::default()
        }
    }

    #[test]
    fn an_empty_context_answers_no_member() {
        let context = CallContext::new();
        for name in ["ehr_id", "patient", "who", "onBehalfOf"] {
            assert_eq!(context.member(name), None, "{name} answered a value");
        }
    }

    #[test]
    fn a_reference_member_answers_its_literal_reference() {
        let context = CallContext::new()
            .with_ehr_id("53d89df2-5501-4455-9a65-565a5d1ddb7c")
            .with_who(reference("Practitioner/456"));
        assert_eq!(
            context.member("ehr_id"),
            Some("53d89df2-5501-4455-9a65-565a5d1ddb7c")
        );
        assert_eq!(context.member("who"), Some("Practitioner/456"));
    }

    #[test]
    fn a_member_the_draft_does_not_declare_answers_none() {
        let context = CallContext::new().with_ehr_id("an-ehr");
        assert_eq!(context.member("composer"), None);
    }
}
