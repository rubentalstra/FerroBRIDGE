// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authoring the one error vocabulary the facade speaks.
//!
//! Everything the facade answers with that is not a FHIR resource is an
//! `OperationOutcome` (<https://hl7.org/fhir/R4/operationoutcome.html>), built
//! from the generated R4 type. An upstream openEHR error body travels inside
//! `issue.diagnostics`, so the two error vocabularies never mix on one wire
//! (`docs/architecture.md` §4.6).
//!
//! [`IssueType`] is the subset of the R4 `issue-type` value set this facade
//! uses, and [`Severity`] the `issue-severity` codes. Both are closed enums,
//! so a handler cannot invent a code the value set does not define.

use fhir_types::codec::Json;
use fhir_types::r4::codeable_concept::CodeableConcept;
use fhir_types::r4::coding::Coding;
use fhir_types::r4::operation_outcome::OperationOutcome;
use fhir_types::r4::operation_outcome::OperationOutcomeIssue;

/// The `issue-severity` code of one issue.
///
/// The value set is `http://hl7.org/fhir/ValueSet/issue-severity`, required on
/// `OperationOutcome.issue.severity`
/// (<https://hl7.org/fhir/R4/operationoutcome.html>).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The action failed.
    Error,
    /// The action succeeded and something is worth reporting.
    Warning,
    /// The issue carries no problem.
    Information,
}

impl Severity {
    /// Returns the code as the value set spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
        }
    }
}

/// The `issue-type` code of one issue.
///
/// Every variant is a code of `http://hl7.org/fhir/ValueSet/issue-type`
/// (<https://hl7.org/fhir/R4/valueset-issue-type.html>), which R4 binds to
/// `OperationOutcome.issue.code` at strength `required`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueType {
    /// Content invalid against the specification.
    Invalid,
    /// A structural issue in the content.
    Structure,
    /// The content is required and was not supplied.
    Required,
    /// A value is out of range or otherwise not allowed.
    Value,
    /// The client is not authenticated.
    Login,
    /// The user or system is not authorized.
    Forbidden,
    /// The reference points at something that does not exist.
    NotFound,
    /// The resource or element has been deleted.
    Deleted,
    /// The interaction, operation or resource is not supported.
    NotSupported,
    /// A duplicate would be created.
    Duplicate,
    /// Two updates collided.
    Conflict,
    /// A required precondition failed.
    BusinessRule,
    /// A transient issue: the operation may succeed on a retry.
    Transient,
    /// An upstream system did not answer usefully.
    Exception,
    /// The processing failed for a reason with no better code.
    Processing,
    /// The operation ran and a note is worth recording.
    Informational,
}

impl IssueType {
    /// Returns the code as the value set spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::Structure => "structure",
            Self::Required => "required",
            Self::Value => "value",
            Self::Login => "login",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not-found",
            Self::Deleted => "deleted",
            Self::NotSupported => "not-supported",
            Self::Duplicate => "duplicate",
            Self::Conflict => "conflict",
            Self::BusinessRule => "business-rule",
            Self::Transient => "transient",
            Self::Exception => "exception",
            Self::Processing => "processing",
            Self::Informational => "informational",
        }
    }
}

/// The code system `OperationOutcome.issue.code` is drawn from.
const ISSUE_TYPE_SYSTEM: &str = "http://terminology.hl7.org/CodeSystem/operation-outcome";

/// One issue under construction.
#[derive(Debug, Clone)]
pub struct Issue {
    /// How serious the issue is.
    severity: Severity,
    /// Which issue type it is.
    code: IssueType,
    /// The human-readable detail, including any upstream error body.
    diagnostics: Option<String>,
    /// A further description of the issue, as a coded detail.
    details: Option<String>,
    /// The element paths the issue is about.
    locations: Vec<String>,
}

impl Issue {
    /// Returns an `error` issue of `code`.
    #[must_use]
    pub const fn error(code: IssueType) -> Self {
        Self::of(Severity::Error, code)
    }

    /// Returns a `warning` issue of `code`.
    #[must_use]
    pub const fn warning(code: IssueType) -> Self {
        Self::of(Severity::Warning, code)
    }

    /// Returns an `information` issue of `code`.
    #[must_use]
    pub const fn information(code: IssueType) -> Self {
        Self::of(Severity::Information, code)
    }

    /// Returns an issue of `severity` and `code`.
    #[must_use]
    pub const fn of(severity: Severity, code: IssueType) -> Self {
        Self {
            severity,
            code,
            diagnostics: None,
            details: None,
            locations: Vec::new(),
        }
    }

    /// Returns this issue with `text` as its `diagnostics`.
    #[must_use]
    pub fn diagnosing(mut self, text: impl Into<String>) -> Self {
        self.diagnostics = Some(text.into());
        self
    }

    /// Returns this issue with `text` as its `details.text`.
    #[must_use]
    pub fn detailing(mut self, text: impl Into<String>) -> Self {
        self.details = Some(text.into());
        self
    }

    /// Returns this issue located at the element path `path`.
    #[must_use]
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.locations.push(path.into());
        self
    }

    /// Returns the severity of this issue.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Returns the issue type of this issue.
    #[must_use]
    pub const fn code(&self) -> IssueType {
        self.code
    }

    /// Returns the generated R4 issue this describes.
    #[must_use]
    pub fn build(&self) -> OperationOutcomeIssue {
        OperationOutcomeIssue {
            severity: self.severity.as_str().into(),
            code: self.code.as_str().into(),
            details: self.details.as_ref().map(|text| CodeableConcept {
                coding: vec![Coding {
                    system: Some(ISSUE_TYPE_SYSTEM.into()),
                    code: Some(self.code.as_str().into()),
                    ..Coding::default()
                }],
                text: Some(text.as_str().into()),
                ..CodeableConcept::default()
            }),
            diagnostics: self.diagnostics.as_ref().map(|text| text.as_str().into()),
            location: self
                .locations
                .iter()
                .map(|path| path.as_str().into())
                .collect(),
            ..OperationOutcomeIssue::default()
        }
    }
}

/// Returns the `OperationOutcome` holding `issues`, in the order given.
#[must_use]
pub fn outcome(issues: &[Issue]) -> OperationOutcome {
    OperationOutcome {
        issue: issues.iter().map(Issue::build).collect(),
        ..OperationOutcome::default()
    }
}

/// Returns the `OperationOutcome` JSON of `issues`.
///
/// # Errors
///
/// Returns [`fhir_types::codec::EncodeError`] when an issue holds a value the
/// FHIR JSON representation cannot carry.
pub fn encoded(
    issues: &[Issue],
) -> Result<fhir_types::codec::Object, fhir_types::codec::EncodeError> {
    outcome(issues).to_json()
}

#[cfg(test)]
mod tests {
    use super::{Issue, IssueType, Severity, encoded, outcome};
    use fhir_types::codec::Value;

    #[test]
    fn every_issue_code_is_the_value_set_spelling() {
        assert_eq!("not-supported", IssueType::NotSupported.as_str());
        assert_eq!("not-found", IssueType::NotFound.as_str());
        assert_eq!("business-rule", IssueType::BusinessRule.as_str());
        assert_eq!("error", Severity::Error.as_str());
    }

    #[test]
    fn an_outcome_carries_its_issues_in_order() {
        let built = outcome(&[
            Issue::error(IssueType::Invalid).diagnosing("the first"),
            Issue::warning(IssueType::Processing).diagnosing("the second"),
        ]);
        assert_eq!(2, built.issue.len());
        assert_eq!(
            Some("the first"),
            built
                .issue
                .first()
                .and_then(|issue| issue.diagnostics.as_ref())
                .and_then(|text| text.value.as_deref())
        );
    }

    #[test]
    fn the_encoded_outcome_names_its_resource_type_and_issue_fields() {
        let object = encoded(&[Issue::error(IssueType::NotFound)
            .diagnosing("no mapping knows this id")
            .at("Condition.id")])
        .expect("an outcome of plain strings encodes");
        assert_eq!(
            Some(&Value::String(String::from("OperationOutcome"))),
            object.get("resourceType")
        );
        let issues = object
            .get("issue")
            .and_then(Value::as_array)
            .expect("the outcome carries an issue array");
        let first = issues.first().expect("one issue");
        assert_eq!(Some("error"), first.get("severity").and_then(Value::as_str));
        assert_eq!(Some("not-found"), first.get("code").and_then(Value::as_str));
        assert_eq!(
            Some("no mapping knows this id"),
            first.get("diagnostics").and_then(Value::as_str)
        );
    }
}
