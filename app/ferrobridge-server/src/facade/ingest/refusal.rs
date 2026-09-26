// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals the service answers with for what the CDR, the EHR
//! resolution and the identity map refuse.

use http::StatusCode;

use crate::cdr::error::CdrError;
use crate::cdr::ids::ContributionUid;
use crate::facade::ehr;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::outcome::chain;
use crate::facade::status;

use super::Mapped;
use super::Refused;

/// Returns the refusal a CDR call that reached no usable documented answer
/// renders as, through the status table.
pub(super) fn cdr_refusal(error: &CdrError) -> Refused {
    Refused::of_answer(&status::of_client_error(error))
}

/// Returns the refusal a documented CDR `answered` refusal with `body` decides
/// through `row`.
pub(super) fn upstream_refusal(
    row: status::Row,
    answered: StatusCode,
    body: &openehr_its::rest::client::ErrorBody,
) -> Refused {
    Refused::of_answer(&status::Answer::new(
        row,
        status::diagnostics(answered, body),
    ))
}

/// Returns the refusal a rejected contribution renders as.
///
/// Nothing was committed, so the answer names every entry the Bundle mapped:
/// a caller cannot tell from a partial list which entries still stand.
pub(super) fn refuse_all(mapped: &[Mapped<'_>], row: &status::Row, detail: &str) -> Refused {
    let mut issues = vec![
        Issue::error(row.issue())
            .diagnosing(detail.to_owned())
            .detailing("the CDR refused the contribution, so no entry of this Bundle is stored"),
    ];
    for entry in mapped {
        issues.push(
            Issue::error(row.issue())
                .diagnosing(detail.to_owned())
                .at(entry.full_url.clone()),
        );
    }
    Refused::of(row.status(), issues)
}

/// Returns the refusal an EHR resolution of a single write renders as.
pub(super) fn ehr_refusal(error: &ehr::EhrError) -> Refused {
    match *error {
        ehr::EhrError::Absent { .. } => Refused::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::NotFound).diagnosing(chain(error)),
        ),
        ehr::EhrError::Refused { .. } | ehr::EhrError::Subject { .. } => Refused::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::Processing).diagnosing(chain(error)),
        ),
        ehr::EhrError::Client { ref source } => {
            Refused::of_answer(&status::of_client_error(source))
        }
        _ => Refused::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(chain(error)),
        ),
    }
}

/// Returns the refusal a malformed stored openEHR identifier renders as.
pub(super) fn stored_identifier(error: &impl std::fmt::Display) -> Refused {
    Refused::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(format!(
            "the identity map holds an openEHR identifier this version cannot read: {error}"
        )),
    )
}

/// Returns the refusal of a committed contribution whose answer cannot bind
/// its entries, naming the contribution so it can be reconciled.
pub(super) fn unbound(contribution: &ContributionUid, why: &str) -> Refused {
    Refused::of_answer(&status::Answer::new(
        status::UNDOCUMENTED,
        format!(
            "the CDR committed contribution {contribution} and {why}, so its entries cannot be bound to their compositions"
        ),
    ))
}
