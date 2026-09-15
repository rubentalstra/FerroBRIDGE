// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Selecting the compiled program one request runs.
//!
//! A FHIR instance names the profiles it claims in `meta.profile`, a list of
//! canonical URLs that may each carry a version after a `|`
//! (<https://hl7.org/fhir/R4/resource.html#Meta>,
//! <https://hl7.org/fhir/R4/references.html#canonical>), so the FHIR side
//! selects by set membership. The openEHR side selects by the template
//! identifier a composition names. When several programs answer, the caller
//! pins the choice with a template identifier, and an unpinned ambiguity is a
//! refusal that names the candidates. FHIRconnect fixes no selection rule, so
//! everything but the two identifiers read here is FerroBRIDGE's own design.

use std::sync::Arc;

use crate::resolve::program::Program;
use crate::resolve::program::TemplateId;

/// Why no single program answers a selection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    /// No compiled program claims the profile or the template.
    #[error("no compiled program answers {wanted}")]
    NoMatch {
        /// What was asked for.
        wanted: String,
    },
    /// More than one program answers and nothing pins the choice.
    #[error("{} programs answer {wanted}: {}", candidates.len(), candidates.join(", "))]
    Ambiguous {
        /// What was asked for.
        wanted: String,
        /// The context name and template of each candidate.
        candidates: Vec<String>,
    },
    /// Every program claiming the profile pins another version.
    #[error(
        "no compiled program answers {wanted}; {} claim a profile with another version: {}",
        mismatches.len(),
        mismatches.join(", ")
    )]
    ProfileVersion {
        /// What was asked for.
        wanted: String,
        /// One `claimed versus pinned` line per program that claims the
        /// profile, in program order.
        mismatches: Vec<String>,
    },
}

/// Returns the program whose profile the instance claims.
///
/// `claimed` is the instance's `meta.profile`, each entry a canonical URL with
/// an optional `|version`.
///
/// # Errors
///
/// Returns [`SelectError::NoMatch`] when no program claims one of the
/// profiles, [`SelectError::Ambiguous`] when more than one does, and
/// [`SelectError::ProfileVersion`], naming every program that claims the
/// profile under another version, when that is why nothing matched.
pub fn select_by_profile<'a>(
    programs: &'a [Arc<Program>],
    claimed: &[String],
) -> Result<&'a Arc<Program>, SelectError> {
    select_by_profile_pinned(programs, claimed, None)
}

/// Returns the program whose profile the instance claims, pinned by template.
///
/// The `templateId` parameter of the FHIRconnect operations is what pins the
/// choice when several contexts map one profile.
///
/// # Errors
///
/// The errors of [`select_by_profile`].
pub fn select_by_profile_pinned<'a>(
    programs: &'a [Arc<Program>],
    claimed: &[String],
    template: Option<&TemplateId>,
) -> Result<&'a Arc<Program>, SelectError> {
    let wanted = format!("the profiles [{}]", claimed.join(", "));
    let mut found: Vec<&'a Arc<Program>> = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();
    for program in programs {
        let url = program.profile().url().as_str();
        let Some(entry) = claimed.iter().find(|entry| profile_url(entry) == url) else {
            continue;
        };
        if let (Some(claimed_version), Some(pinned)) = (
            profile_version(entry),
            program.profile().version().version(),
        ) && claimed_version != pinned
        {
            mismatches.push(format!(
                "{} claims `{claimed_version}` and pins `{pinned}`",
                program.context()
            ));
            continue;
        }
        found.push(program);
    }
    narrow(found, template, wanted).map_err(|error| match error {
        SelectError::NoMatch { wanted } if !mismatches.is_empty() => {
            SelectError::ProfileVersion { wanted, mismatches }
        }
        error => error,
    })
}

/// Returns the program compiled against the template a composition names.
///
/// # Errors
///
/// Returns [`SelectError::NoMatch`] when no program was compiled against the
/// template and [`SelectError::Ambiguous`] when more than one was.
pub fn select_by_template<'a>(
    programs: &'a [Arc<Program>],
    template: &TemplateId,
) -> Result<&'a Arc<Program>, SelectError> {
    let found: Vec<&'a Arc<Program>> = programs
        .iter()
        .filter(|program| program.template().id() == template)
        .collect();
    narrow(found, None, format!("the template `{template}`"))
}

/// Narrows a candidate set to one program, with the template as the pin.
fn narrow<'a>(
    found: Vec<&'a Arc<Program>>,
    template: Option<&TemplateId>,
    wanted: String,
) -> Result<&'a Arc<Program>, SelectError> {
    let narrowed: Vec<&'a Arc<Program>> = match template {
        Some(pin) => found
            .into_iter()
            .filter(|program| program.template().id() == pin)
            .collect(),
        None => found,
    };
    match *narrowed.as_slice() {
        [only] => Ok(only),
        [] => Err(SelectError::NoMatch { wanted }),
        _ => Err(SelectError::Ambiguous {
            wanted,
            candidates: narrowed.iter().map(|program| describe(program)).collect(),
        }),
    }
}

/// Names one candidate program for a refusal.
fn describe(program: &Program) -> String {
    format!("{} ({})", program.context(), program.template().id())
}

/// Returns the URL half of a canonical reference.
fn profile_url(canonical: &str) -> &str {
    canonical.split('|').next().unwrap_or(canonical)
}

/// Returns the version half of a canonical reference, when it carries one.
fn profile_version(canonical: &str) -> Option<&str> {
    let mut parts = canonical.splitn(2, '|');
    let _url = parts.next();
    parts.next().filter(|version| !version.is_empty())
}
