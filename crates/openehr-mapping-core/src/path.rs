// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR path model the two mapping languages share.
//!
//! A path in a mapping file is an openEHR RM path with two additions the
//! languages make for themselves. It may open with a variable
//! (`$archetype/data[at0001]`, `$openehrRoot/defining_code`), and it may open
//! with one or more `../` parent steps
//! (`docs/specs/fhirconnect/modules/ROOT/pages/basics/path_operators.adoc`).
//! Neither is openEHR: the path grammar of openEHR BASE Release 1.2.0
//! §Paths and Locators
//! (<https://specifications.openehr.org/releases/BASE/Release-1.2.0/architecture_overview.html>)
//! defines absolute paths, relative paths, predicates and the `//` pattern,
//! and no parent step.
//!
//! The tail of a path is parsed by the BASE path parser of `openehr-rm`
//! (<https://docs.rs/openehr-rm/0.0.69/openehr_rm/v1_2/paths/struct.RmPath.html>),
//! so this module never re-implements the openEHR grammar. It resolves the
//! parent steps against an anchor before the path is used, and the resolved
//! path carries no `..` segment.

use core::fmt;
use core::str::FromStr;

use openehr_rm::v1_2::paths::PathError;
use openehr_rm::v1_2::paths::RmPath;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticCode;

/// The parent step both mapping languages write.
const PARENT_STEP: &str = "..";

/// A `$name` variable at the head of a mapping path.
///
/// The variables FHIRconnect defines are `$resource`, `$fhirRoot`,
/// `$archetype`, `$openehrRoot`, `$composition`, `$reference` and `$context`
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`). This
/// type keeps the name and interprets none of them: which openEHR path a
/// variable stands for is the language crate's decision, and it reaches
/// [`MappingPath::resolve`] as the anchor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathVariable(String);

impl PathVariable {
    /// Creates a path variable from its name, without the leading `$`.
    ///
    /// # Errors
    ///
    /// Returns [`PathVariableError`] when the name is empty or is not a letter
    /// followed by letters and digits.
    pub fn new(name: impl Into<String>) -> Result<Self, PathVariableError> {
        let name = name.into();
        let mut characters = name.chars();
        let Some(first) = characters.next() else {
            return Err(PathVariableError::Empty);
        };
        if !first.is_ascii_alphabetic() || !characters.all(|c| c.is_ascii_alphanumeric()) {
            return Err(PathVariableError::Name { name });
        }
        Ok(Self(name))
    }

    /// Returns the name, without the leading `$`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PathVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}", self.0)
    }
}

/// Why a `$name` variable was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathVariableError {
    /// The `$` was followed by nothing.
    #[error("a path variable is `$` followed by a name")]
    Empty,
    /// The name is not a letter followed by letters and digits.
    #[error("the path variable name `{name}` is not a letter followed by letters and digits")]
    Name {
        /// The refused name.
        name: String,
    },
}

/// Why a mapping path was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MappingPathError {
    /// The path was empty.
    #[error("a mapping path is not empty")]
    Empty,
    /// The head variable was refused.
    #[error("the head of `{path}` was refused")]
    Variable {
        /// The refused path.
        path: String,
        /// Why the variable was refused.
        #[source]
        source: PathVariableError,
    },
    /// A `..` step appears after the head of the path.
    #[error("`{path}` writes a `..` step after the head of the path")]
    InteriorParentStep {
        /// The refused path.
        path: String,
    },
    /// The openEHR tail was refused by the BASE path parser.
    #[error("the openEHR tail of `{path}` was refused")]
    Rm {
        /// The refused path.
        path: String,
        /// Why the tail was refused.
        #[source]
        source: PathError,
    },
}

impl MappingPathError {
    /// Returns the code this refusal reports.
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        match *self {
            Self::Empty
            | Self::Variable { .. }
            | Self::InteriorParentStep { .. }
            | Self::Rm { .. } => DiagnosticCode::MalformedPath,
        }
    }

    /// Renders this refusal as a diagnostic about `file`.
    #[must_use]
    pub fn to_diagnostic(&self, file: impl Into<std::path::PathBuf>) -> Diagnostic {
        Diagnostic::error(file, self.code(), self.to_string())
    }
}

/// Why a mapping path could not be resolved against an anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathResolutionError {
    /// The path walks above the root of the anchor it resolves against.
    #[error("the path walks up {steps} times from an anchor {anchor_depth} segments deep")]
    AboveAnchorRoot {
        /// How many parent steps the path writes.
        steps: usize,
        /// How many segments the anchor has.
        anchor_depth: usize,
    },
}

impl PathResolutionError {
    /// Returns the code this refusal reports.
    #[must_use]
    pub const fn code(self) -> DiagnosticCode {
        match self {
            Self::AboveAnchorRoot { .. } => DiagnosticCode::PathAboveAnchorRoot,
        }
    }

    /// Renders this refusal as a diagnostic about `file`.
    #[must_use]
    pub fn to_diagnostic(self, file: impl Into<std::path::PathBuf>) -> Diagnostic {
        Diagnostic::error(file, self.code(), self.to_string())
    }
}

/// A path as a mapping file writes it.
///
/// The three parts are the optional head variable, the number of `../` parent
/// steps, and the openEHR tail. [`MappingPath::resolve`] turns the first two
/// into an anchored [`RmPath`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingPath {
    variable: Option<PathVariable>,
    parent_steps: usize,
    tail: RmPath,
}

impl MappingPath {
    /// Returns the head variable, when the path opens with one.
    #[must_use]
    pub const fn variable(&self) -> Option<&PathVariable> {
        self.variable.as_ref()
    }

    /// Returns how many `../` parent steps the path opens with.
    #[must_use]
    pub const fn parent_steps(&self) -> usize {
        self.parent_steps
    }

    /// Returns the openEHR tail, which carries no parent step.
    #[must_use]
    pub const fn tail(&self) -> &RmPath {
        &self.tail
    }

    /// Resolves this path against the anchor it is written relative to.
    ///
    /// The anchor is the path the head stands for: what the variable names, or
    /// the enclosing mapping's own path for a bare relative path. Each parent
    /// step drops one segment from the anchor, then the tail is appended. The
    /// result carries the anchor's absolute flag and never carries a `..`
    /// segment.
    ///
    /// # Errors
    ///
    /// Returns [`PathResolutionError::AboveAnchorRoot`] when the path writes
    /// more parent steps than the anchor has segments.
    pub fn resolve(&self, anchor: &RmPath) -> Result<RmPath, PathResolutionError> {
        let anchor_depth = anchor.segments.len();
        if self.parent_steps > anchor_depth {
            return Err(PathResolutionError::AboveAnchorRoot {
                steps: self.parent_steps,
                anchor_depth,
            });
        }
        let kept = anchor_depth - self.parent_steps;
        let mut segments: Vec<_> = anchor.segments.iter().take(kept).cloned().collect();
        segments.extend(self.tail.segments.iter().cloned());
        Ok(RmPath {
            absolute: anchor.absolute,
            segments,
        })
    }
}

impl fmt::Display for MappingPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut written = false;
        if let Some(ref variable) = self.variable {
            write!(f, "{variable}")?;
            written = true;
        }
        for _ in 0..self.parent_steps {
            if written {
                f.write_str("/")?;
            }
            f.write_str(PARENT_STEP)?;
            written = true;
        }
        if self.tail.segments.is_empty() {
            return Ok(());
        }
        if written {
            f.write_str("/")?;
        }
        write!(f, "{}", self.tail)
    }
}

impl FromStr for MappingPath {
    type Err = MappingPathError;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        if path.is_empty() {
            return Err(MappingPathError::Empty);
        }

        let (variable, mut rest) = match path.strip_prefix('$') {
            Some(after) => {
                let (name, rest) = match after.split_once('/') {
                    Some((name, rest)) => (name, rest),
                    None => (after, ""),
                };
                let variable =
                    PathVariable::new(name).map_err(|source| MappingPathError::Variable {
                        path: path.to_owned(),
                        source,
                    })?;
                (Some(variable), rest)
            }
            None => (None, path),
        };

        let mut parent_steps = 0_usize;
        loop {
            if let Some(after) = rest.strip_prefix("../") {
                parent_steps = parent_steps.saturating_add(1);
                rest = after;
                continue;
            }
            if rest == PARENT_STEP {
                parent_steps = parent_steps.saturating_add(1);
                rest = "";
            }
            break;
        }

        let tail = if rest.is_empty() {
            RmPath {
                absolute: false,
                segments: Vec::new(),
            }
        } else {
            RmPath::from_str(rest).map_err(|source| MappingPathError::Rm {
                path: path.to_owned(),
                source,
            })?
        };

        if tail
            .segments
            .iter()
            .any(|segment| segment.attribute == PARENT_STEP)
        {
            return Err(MappingPathError::InteriorParentStep {
                path: path.to_owned(),
            });
        }

        Ok(Self {
            variable,
            parent_steps,
            tail,
        })
    }
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use openehr_rm::v1_2::paths::PathError;
    use openehr_rm::v1_2::paths::RmPath;

    use super::MappingPath;
    use super::MappingPathError;
    use super::PathResolutionError;

    fn anchor(path: &str) -> RmPath {
        RmPath::from_str(path).expect("a well-formed anchor")
    }

    #[test]
    fn a_variable_head_is_kept_opaque() {
        let path = MappingPath::from_str("$archetype/data[at0001]/items[at0077]")
            .expect("a well-formed path");
        assert_eq!(
            path.variable().map(|v| v.name().to_owned()),
            Some("archetype".to_owned())
        );
        assert_eq!(path.parent_steps(), 0);
        assert_eq!(path.tail().segments.len(), 2);
        assert_eq!(path.to_string(), "$archetype/data[at0001]/items[at0077]");
    }

    #[test]
    fn a_variable_alone_has_an_empty_tail() {
        let path = MappingPath::from_str("$reference").expect("a well-formed path");
        assert_eq!(path.tail().segments.len(), 0);
        assert_eq!(path.to_string(), "$reference");
    }

    #[test]
    fn parent_steps_resolve_against_the_anchor() {
        let path = MappingPath::from_str("../../items[at0001]/value").expect("a well-formed path");
        assert_eq!(path.parent_steps(), 2);
        let resolved = path
            .resolve(&anchor(
                "/content[openEHR-EHR-OBSERVATION.body_weight.v2]/data[at0002]/events[at0003]",
            ))
            .expect("the anchor is deep enough");
        assert_eq!(
            resolved.to_string(),
            "/content[openEHR-EHR-OBSERVATION.body_weight.v2]/items[at0001]/value"
        );
        assert!(!resolved.to_string().contains(".."));
    }

    #[test]
    fn a_bare_parent_run_resolves_to_the_anchor_prefix() {
        let path = MappingPath::from_str("../../").expect("a well-formed path");
        assert_eq!(path.parent_steps(), 2);
        assert_eq!(path.tail().segments.len(), 0);
        let resolved = path
            .resolve(&anchor("/content[at0001]/data[at0002]/items[at0003]"))
            .expect("the anchor is deep enough");
        assert_eq!(resolved.to_string(), "/content[at0001]");
    }

    #[test]
    fn one_parent_step_too_many_is_a_typed_error() {
        let path = MappingPath::from_str("../../../value").expect("a well-formed path");
        assert_eq!(
            path.resolve(&anchor("/data[at0001]/items[at0002]")),
            Err(PathResolutionError::AboveAnchorRoot {
                steps: 3,
                anchor_depth: 2,
            })
        );
    }

    #[test]
    fn a_zero_positional_predicate_is_refused() {
        let error = MappingPath::from_str("items[0]").expect_err("a 0 index");
        assert!(
            matches!(
                error,
                MappingPathError::Rm {
                    source: PathError::InvalidPosition(_),
                    ..
                }
            ),
            "{error:?}"
        );
        assert!(MappingPath::from_str("items[1]").is_ok());
    }

    #[test]
    fn an_interior_parent_step_is_refused() {
        assert_eq!(
            MappingPath::from_str("items[at0001]/../value"),
            Err(MappingPathError::InteriorParentStep {
                path: "items[at0001]/../value".to_owned()
            })
        );
    }

    #[test]
    fn node_id_and_name_predicates_survive_the_parser() {
        let path = MappingPath::from_str(
            "$archetype/items[at0001 and name/value='systolic']/content[openEHR-EHR-EVALUATION.gender.v1]",
        )
        .expect("a well-formed path");
        let first = path.tail().segments.first().expect("a first segment");
        assert_eq!(first.predicate.archetype_node_id.as_deref(), Some("at0001"));
        assert_eq!(first.predicate.name_value.as_deref(), Some("systolic"));
        let second = path.tail().segments.get(1).expect("a second segment");
        assert_eq!(
            second.predicate.archetype_node_id.as_deref(),
            Some("openEHR-EHR-EVALUATION.gender.v1")
        );
    }

    #[test]
    fn an_empty_path_is_refused() {
        assert_eq!(MappingPath::from_str(""), Err(MappingPathError::Empty));
    }
}
