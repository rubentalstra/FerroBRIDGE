// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The engine's declared losses as `OperationOutcome` issues.
//!
//! "Where an engine returns a partial result rather than failing outright ...
//! those gaps MUST be made explicit through the `OperationOutcome` rather than
//! silently dropping data" (`engine/rest-api.adoc`
//! §Reporting partial results with `OperationOutcome`). FerroBRIDGE never
//! returns a partial result, so what reaches an outcome here is the closed set
//! of losses [`crate::engine::outcome::Warning`] declares, and nothing else.
//!
//! A composition field the engine filled is `information`, because the run
//! produced a value the source did not carry. Everything else is `warning`,
//! because content the source carried did not reach the output. The draft
//! codes its own worked example `incomplete`, which is the type used here.

use fhir_types::r4::codeable_concept::CodeableConcept;
use fhir_types::r4::operation_outcome::OperationOutcome;
use fhir_types::r4::operation_outcome::OperationOutcomeIssue;
use fhir_types::r4::primitives::Code;

use crate::engine::outcome::Warning;

/// The `error` severity of `IssueSeverity`.
pub const ERROR: &str = "error";

/// The `warning` severity of `IssueSeverity`.
pub const WARNING: &str = "warning";

/// The `information` severity of `IssueSeverity`.
pub const INFORMATION: &str = "information";

/// Returns one issue with `severity`, `code` and `text`.
///
/// The severities are the `IssueSeverity` codes
/// (<https://hl7.org/fhir/R4/valueset-issue-severity.html>) and the codes the
/// `IssueType` ones (<https://hl7.org/fhir/R4/valueset-issue-type.html>).
#[must_use]
pub fn issue(severity: &str, code: &str, text: &str) -> OperationOutcomeIssue {
    OperationOutcomeIssue {
        severity: Code::from(severity),
        code: Code::from(code),
        details: Some(CodeableConcept {
            text: Some(fhir_types::r4::primitives::String::from(text)),
            ..CodeableConcept::default()
        }),
        ..OperationOutcomeIssue::default()
    }
}

/// Returns the issue one declared loss renders as.
#[must_use]
pub fn of_warning(warning: &Warning) -> OperationOutcomeIssue {
    let severity = match *warning {
        Warning::Defaulted { .. } => INFORMATION,
        Warning::Skipped { .. }
        | Warning::LastOfMany { .. }
        | Warning::OneWayFallback { .. }
        | Warning::DeferredReference { .. } => WARNING,
    };
    let code = match *warning {
        Warning::Defaulted { .. } => "informational",
        Warning::Skipped { .. }
        | Warning::LastOfMany { .. }
        | Warning::OneWayFallback { .. }
        | Warning::DeferredReference { .. } => "incomplete",
    };
    issue(severity, code, &warning.to_string())
}

/// Returns the outcome `warnings` render as, or `None` when there are none.
///
/// A run that declared no loss answers no outcome at all: the draft's
/// `outcome` parameter is "Present when mapping issues occurred".
#[must_use]
pub fn of_warnings(warnings: &[Warning]) -> Option<OperationOutcome> {
    if warnings.is_empty() {
        return None;
    }
    Some(OperationOutcome {
        issue: warnings.iter().map(of_warning).collect(),
        ..OperationOutcome::default()
    })
}

#[cfg(test)]
mod tests {
    use super::of_warning;
    use super::of_warnings;
    use crate::engine::outcome::SkipReason;
    use crate::engine::outcome::Warning;

    #[test]
    fn no_loss_renders_no_outcome() {
        assert!(of_warnings(&[]).is_none());
    }

    #[test]
    fn a_defaulted_field_is_information() {
        let issue = of_warning(&Warning::Defaulted {
            field: String::from("context/start_time"),
        });
        assert_eq!(issue.severity.value.as_deref(), Some("information"));
        assert_eq!(issue.code.value.as_deref(), Some("informational"));
    }

    #[test]
    fn a_skipped_mapping_is_an_incomplete_warning() {
        let issue = of_warning(&Warning::Skipped {
            mapping: String::from("bodySite"),
            reason: SkipReason::Unidirectional,
        });
        assert_eq!(issue.severity.value.as_deref(), Some("warning"));
        assert_eq!(issue.code.value.as_deref(), Some("incomplete"));
    }
}
