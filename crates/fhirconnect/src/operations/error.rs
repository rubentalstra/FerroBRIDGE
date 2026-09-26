// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Why an operation refused, and the `OperationOutcome` it answers with.
//!
//! Every refusal carries the R4 issue type it renders as
//! (<https://hl7.org/fhir/R4/valueset-issue-type.html>) and its cause, so a
//! caller branches on the variant and a person reads the outcome.

use fhir_types::codec::DecodeError;
use fhir_types::operation::ParametersError;
use fhir_types::r4::operation_outcome::OperationOutcome;
use openehr_mapping_core::template::PathError;

use crate::engine::traverse::error::EngineError;
use crate::operations::issues;
use crate::resolve::select::SelectError;

/// Why an operation did not answer.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    /// The `Parameters` resource does not fit the declared parameter set.
    #[error("the ${operation} request does not fit its declared parameters")]
    Parameters {
        /// The operation code, without the `$`.
        operation: &'static str,
        /// Which parameter was refused and why.
        #[source]
        source: ParametersError,
    },
    /// The body is not the FHIR resource the operation takes.
    #[error("the ${operation} body is not a {expected}")]
    Body {
        /// The operation code, without the `$`.
        operation: &'static str,
        /// The resource type the operation takes.
        expected: &'static str,
        /// Why the strict codec refused it.
        #[source]
        source: Box<DecodeError>,
    },
    /// The composition string is not a JSON object.
    #[error("the composition parameter does not hold a JSON object")]
    Payload {
        /// Why the reader refused it.
        #[source]
        source: serde_json::Error,
    },
    /// A body that must be a JSON object is not one.
    #[error("the {what} is not a JSON object")]
    NotAnObject {
        /// What was read.
        what: &'static str,
    },
    /// A resource of the model does not render as JSON.
    #[error("a resource of the bundle does not render as JSON")]
    Encode {
        /// Why the encoder refused it.
        #[source]
        source: fhir_types::codec::EncodeError,
    },
    /// A FLAT composition arrived without `templateId`.
    ///
    /// "`templateId` ... is also **required** when submitting a flat
    /// Composition to `$tofhir`, because a flat payload does not carry its
    /// template inline" (`engine/rest-api.adoc` §Query parameters).
    #[error("a flat composition carries no template, so `templateId` is required")]
    FlatTemplate,
    /// A canonical composition names no template and none was pinned.
    #[error(
        "the composition names no `archetype_details.template_id`, so `templateId` is required"
    )]
    CanonicalTemplate,
    /// The payload and the parameter name different templates.
    #[error("the composition names the template `{payload}` and the request pins `{pinned}`")]
    TemplateDisagreement {
        /// The template the composition carries inline.
        payload: String,
        /// The template the request pinned.
        pinned: String,
    },
    /// No template of that identifier is loaded.
    #[error("no operational template `{template}` is loaded")]
    UnknownTemplate {
        /// The template the request asked for.
        template: String,
    },
    /// No single compiled program answers the request.
    #[error("no single mapping answers this request")]
    Select {
        /// Why the selection did not narrow to one program.
        #[source]
        source: SelectError,
    },
    /// The bundle carries no resource of the type the program maps.
    #[error("the bundle carries no {resource} for the mapping `{context}`")]
    NoSubjectResource {
        /// The resource type the program maps.
        resource: String,
        /// The context mapping the program compiled from.
        context: String,
    },
    /// The bundle carries more than one resource of the mapped type.
    #[error("the bundle carries {count} {resource} resources, and one run maps one")]
    SeveralSubjectResources {
        /// The resource type the program maps.
        resource: String,
        /// How many the bundle carries.
        count: usize,
    },
    /// The bundle references more than one subject.
    ///
    /// One Bundle maps to one composition, and a composition belongs to one
    /// EHR, so a mixed-subject Bundle cannot be one composition.
    #[error("the bundle references {} subjects: {}", subjects.len(), subjects.join(", "))]
    SeveralSubjects {
        /// Every distinct subject the bundle names, in order.
        subjects: Vec<String>,
    },
    /// The mapping did not run to completion.
    #[error("the mapping did not run to completion")]
    Mapping {
        /// The element the engine refused.
        #[source]
        source: Box<EngineError>,
    },
    /// The composition could not be converted between its two serializations.
    #[error("the composition could not be converted between its serializations")]
    Serialization {
        /// Why the conversion refused.
        #[source]
        source: Box<PathError>,
    },
    /// The mapped resource is not a resource of the FHIR version in play.
    #[error("the mapping produced a document the {version} model does not admit")]
    Mapped {
        /// The FHIR version the model carries.
        version: &'static str,
        /// Why the strict codec refused it.
        #[source]
        source: Box<DecodeError>,
    },
    /// Neither the mapping nor the composition gives the mapped resource an
    /// identity.
    ///
    /// The draft puts a `Provenance` in every `$tofhir` Bundle, and
    /// `Provenance.target` is a reference, so a resource with no identity
    /// cannot be the target of one.
    #[error(
        "the mapped {resource} carries no `id` and the composition carries no `uid`, so the \
         Provenance has nothing to target"
    )]
    Unidentified {
        /// The resource type the program maps.
        resource: String,
    },
}

impl OperationError {
    /// Returns the R4 issue type this refusal renders as.
    ///
    /// The codes are the ones `valueset-issue-type.html` defines
    /// (<https://hl7.org/fhir/R4/valueset-issue-type.html>).
    #[must_use]
    pub fn issue_type(&self) -> &'static str {
        match *self {
            Self::Parameters { ref source, .. } => parameters_issue_type(source),
            Self::Body { .. }
            | Self::Payload { .. }
            | Self::NotAnObject { .. }
            | Self::Encode { .. }
            | Self::Mapped { .. } => "structure",
            Self::FlatTemplate | Self::CanonicalTemplate => "required",
            Self::TemplateDisagreement { .. } => "invalid",
            Self::UnknownTemplate { .. } | Self::NoSubjectResource { .. } => "not-found",
            Self::Select { ref source } => match *source {
                SelectError::Ambiguous { .. } => "multiple-matches",
                SelectError::NoMatch { .. } | SelectError::ProfileVersion { .. } => "not-found",
            },
            Self::SeveralSubjectResources { .. } | Self::SeveralSubjects { .. } => "business-rule",
            Self::Mapping { .. } | Self::Serialization { .. } | Self::Unidentified { .. } => {
                "processing"
            }
        }
    }

    /// Returns the `OperationOutcome` this refusal answers with.
    ///
    /// The issue is one `error`, because strictness is the default: a failed
    /// mapping answers an outcome and no Bundle or composition.
    #[must_use]
    pub fn outcome(&self) -> OperationOutcome {
        OperationOutcome {
            issue: vec![issues::issue(
                issues::ERROR,
                self.issue_type(),
                &self.rendered(),
            )],
            ..OperationOutcome::default()
        }
    }

    /// Returns the text the outcome carries.
    ///
    /// The whole cause chain is rendered, because a refusal naming only its
    /// outermost layer tells a mapping author nothing. The text answers the
    /// caller that sent the content and never reaches a log; "Diagnostic
    /// information" is what `OperationOutcome.details` is for
    /// (<https://hl7.org/fhir/R4/operationoutcome.html>).
    fn rendered(&self) -> String {
        let mut line = self.to_string();
        let mut cause: Option<&dyn core::error::Error> = core::error::Error::source(self);
        while let Some(source) = cause {
            line.push_str(": ");
            line.push_str(&source.to_string());
            cause = source.source();
        }
        line
    }
}

/// Returns the R4 issue type one parameter refusal renders as.
fn parameters_issue_type(error: &ParametersError) -> &'static str {
    match *error {
        ParametersError::Unnamed { .. }
        | ParametersError::Undeclared { .. }
        | ParametersError::Repeated { .. } => "structure",
        ParametersError::Missing { .. } | ParametersError::MissingValue { .. } => "required",
        ParametersError::WrongType { .. } => "value",
    }
}

#[cfg(test)]
mod tests {
    use super::OperationError;
    use fhir_types::operation::ParametersError;

    #[test]
    fn a_flat_payload_without_a_template_is_required() {
        let error = OperationError::FlatTemplate;
        assert_eq!(error.issue_type(), "required");
        let outcome = error.outcome();
        assert_eq!(outcome.issue.len(), 1);
        assert_eq!(
            outcome
                .issue
                .first()
                .and_then(|issue| issue.code.value.clone()),
            Some(String::from("required"))
        );
    }

    #[test]
    fn an_undeclared_parameter_is_a_structural_refusal() {
        let error = OperationError::Parameters {
            operation: crate::operations::TO_FHIR,
            source: ParametersError::Undeclared {
                operation: "$tofhir",
                name: String::from("subject"),
            },
        };
        assert_eq!(error.issue_type(), "structure");
        let outcome = error.outcome();
        let text = outcome
            .issue
            .first()
            .and_then(|issue| issue.details.as_ref())
            .and_then(|details| details.text.as_ref())
            .and_then(|text| text.value.clone())
            .unwrap_or_default();
        assert!(
            text.contains("subject"),
            "the outcome names the refused parameter: {text}"
        );
    }
}
