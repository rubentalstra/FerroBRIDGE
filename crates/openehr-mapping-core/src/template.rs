// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The template a path resolves against, in either generation.
//!
//! An operational template reaches the bridge as OPT 1.4 canonical XML or as
//! AOM2 canonical JSON, and the ITS-REST 1.1.0 Definition API serves both
//! (<https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/definition.html>).
//! The Simplified Formats specification defines a Web Template as a processed
//! representation of an operational template without naming a generation, so
//! both generations are built into the one `WebTemplate` the rest of this
//! crate indexes.
//!
//! The Web Template is built here, never fetched: ITS-REST 1.1.0 defines the
//! `application/openehr.wt+json` representation for the `adl1.4` route alone,
//! and the node-id uniqueness rule of the Simplified Formats specification
//! leaves sibling order open, so two servers may name the same node
//! differently.

use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_sdt::flat::error::FlatError;
use openehr_sdt::flat::webtemplate::model::WebTemplate;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticCode;
use crate::path::PathResolutionError;

/// The leader of an ADL 2 id-code (`AM Release 2.4.0`, `adl_code_definitions`
/// §Constants).
const ID_CODE_LEADER: &str = "id";

/// The generation of the template a Web Template was built from.
///
/// The generation is decided by which route served the template, never by a
/// field of the body. It is carried through to every resolved node because two
/// deltas depend on it: a term binding's value is a code for ADL 1.4 and a URI
/// for ADL 2, and constraint bindings exist only on the ADL 1.4 side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Generation {
    /// The AOM 1.4 line: an OPT 1.4 in canonical XML.
    Adl14,
    /// The AM 2.4 line: an OPT2 in AOM2 canonical JSON.
    Adl2,
}

impl Generation {
    /// Returns the stable spelling of the generation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Adl14 => "adl1.4",
            Self::Adl2 => "adl2",
        }
    }
}

impl core::fmt::Display for Generation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An operational template, in the generation it was served in.
///
/// This is the same two-variant shape the ITS-REST client returns. The client
/// owns the fetch and this crate owns the build, so the type is spelled here
/// and the client converts into it.
#[derive(Debug)]
pub enum TemplateSource {
    /// A parsed OPT 1.4, the body of the `adl1.4` route.
    Opt14(Box<openehr_its::opt14::types::OperationalTemplate>),
    /// A parsed OPT2, the body of the `adl2` route.
    Opt2 {
        /// The operational template itself.
        template: Box<OperationalTemplate>,
        /// The full HRID the request resolved to, as the client read it.
        resolved_id: String,
    },
}

impl TemplateSource {
    /// Reads an OPT 1.4 canonical XML document.
    ///
    /// This is the ADL 1.4 line of the two generations, the one an `.opt` file
    /// on disk and the `adl1.4` route of ITS-REST both carry.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::TemplateParse`] when the XML is not an OPT 1.4
    /// operational template.
    pub fn opt14(xml: &str) -> Result<Self, PathError> {
        openehr_its::opt14::from_xml(xml)
            .map(|template| Self::Opt14(Box::new(template)))
            .map_err(|source| PathError::TemplateParse {
                generation: Generation::Adl14,
                source: Box::new(source),
            })
    }

    /// Returns the generation this template was served in.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        match *self {
            Self::Opt14(_) => Generation::Adl14,
            Self::Opt2 { .. } => Generation::Adl2,
        }
    }

    /// Returns the full HRID an ADL 2 fetch resolved to, when there is one.
    ///
    /// An ADL 1.4 template is named by a plain template id and carries no
    /// HRID, so this answers `None` for it.
    #[must_use]
    pub fn resolved_id(&self) -> Option<&str> {
        match *self {
            Self::Opt14(_) => None,
            Self::Opt2 {
                ref resolved_id, ..
            } => Some(resolved_id.as_str()),
        }
    }
}

/// Builds the Web Template of `source` with the builder its generation needs.
///
/// Both builders run the same compaction, in-context synthesis and node-id
/// passes and return the same type, so everything downstream of this function
/// is generation-blind.
///
/// # Errors
///
/// Returns [`PathError::TemplateBuild`] when the builder refuses the
/// operational template.
pub fn web_template(source: &TemplateSource) -> Result<WebTemplate, PathError> {
    let built = match *source {
        TemplateSource::Opt14(ref opt) => {
            openehr_sdt::flat::webtemplate::builder::build_web_template(opt)
        }
        TemplateSource::Opt2 { ref template, .. } => {
            openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4(template)
        }
    };
    built.map_err(|source_error| PathError::TemplateBuild {
        generation: source.generation(),
        source: Box::new(source_error),
    })
}

/// Whether `node_id` is an ADL 2 id-code.
///
/// ADL 2.4 admits at-codes and id-codes as node identifiers and states that
/// "the at-code coding system must be used for systems that need to be
/// conformant to the openEHR Reference Model"
/// (<https://specifications.openehr.org/releases/AM/Release-2.4.0/ADL2.html>,
/// §ADL 2.4). An archetype root carries an archetype id rather than a local
/// code, and neither an at-code nor an archetype id starts with `id` followed
/// by a digit.
#[must_use]
pub(crate) fn is_id_code(node_id: &str) -> bool {
    node_id
        .strip_prefix(ID_CODE_LEADER)
        .is_some_and(|rest| rest.starts_with(|character: char| character.is_ascii_digit()))
}

/// What a coded value a term binding names is.
///
/// The two builders differ here: an ADL 1.4 `TermBindingSet` binds an at-code
/// to a `CODE_PHRASE`, so the bound value is a code in the named terminology,
/// while an AOM2 `ARCHETYPE_TERMINOLOGY.term_bindings` binds a code to a URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindingKind {
    /// The bound value is a code in the terminology the binding names.
    Code,
    /// The bound value is a URI.
    Uri,
}

impl BindingKind {
    /// Returns the kind of bound value the given generation carries.
    #[must_use]
    pub const fn of(generation: Generation) -> Self {
        match generation {
            Generation::Adl14 => Self::Code,
            Generation::Adl2 => Self::Uri,
        }
    }
}

/// One external terminology binding of one Web Template node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The terminology the binding names.
    system: String,
    /// The bound value, read as [`Binding::kind`] says.
    value: String,
    /// What the bound value is.
    kind: BindingKind,
}

impl Binding {
    /// Creates a binding of the given kind.
    #[must_use]
    pub fn new(system: impl Into<String>, value: impl Into<String>, kind: BindingKind) -> Self {
        Self {
            system: system.into(),
            value: value.into(),
            kind,
        }
    }

    /// Returns the terminology the binding names.
    #[must_use]
    pub fn system(&self) -> &str {
        &self.system
    }

    /// Returns the bound value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns what the bound value is.
    #[must_use]
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }
}

/// One constraint binding of one Web Template node.
///
/// An ADL 1.4 ontology binds an `ac` code to a terminology query whose result
/// is the admissible value set (`AM ADL 1.4` §`Constraint_bindings`). AOM2 keeps
/// value sets in the archetype terminology instead, so an ADL 2 template
/// carries none of these and the list is always empty for one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintBinding {
    /// The RM attribute of the leaf carrying the constrained coded value.
    attribute: String,
    /// The archetype constraint code the binding is keyed by.
    ac_code: String,
    /// The terminology the binding names.
    terminology: String,
    /// The terminology query whose result is the admissible value set.
    query_uri: String,
}

impl ConstraintBinding {
    /// Creates a constraint binding.
    #[must_use]
    pub fn new(
        attribute: impl Into<String>,
        ac_code: impl Into<String>,
        terminology: impl Into<String>,
        query_uri: impl Into<String>,
    ) -> Self {
        Self {
            attribute: attribute.into(),
            ac_code: ac_code.into(),
            terminology: terminology.into(),
            query_uri: query_uri.into(),
        }
    }

    /// Returns the RM attribute of the leaf carrying the constrained value.
    #[must_use]
    pub fn attribute(&self) -> &str {
        &self.attribute
    }

    /// Returns the archetype constraint code the binding is keyed by.
    #[must_use]
    pub fn ac_code(&self) -> &str {
        &self.ac_code
    }

    /// Returns the terminology the binding names.
    #[must_use]
    pub fn terminology(&self) -> &str {
        &self.terminology
    }

    /// Returns the terminology query whose result is the admissible value set.
    #[must_use]
    pub fn query_uri(&self) -> &str {
        &self.query_uri
    }
}

/// Why a template, a path or a composition was refused.
#[derive(Debug, thiserror::Error)]
pub enum PathError {
    /// The document is not an operational template of that generation.
    #[error("the document is not an {generation} operational template")]
    TemplateParse {
        /// The generation the document was read as.
        generation: Generation,
        /// Why the reader refused it.
        #[source]
        source: Box<openehr_its::xml::runtime::XmlError>,
    },
    /// The Web Template builder refused the operational template.
    #[error("the {generation} operational template does not build a web template")]
    TemplateBuild {
        /// The generation the template was served in.
        generation: Generation,
        /// Why the builder refused it.
        #[source]
        source: Box<FlatError>,
    },
    /// The template identifies its nodes with ADL 2 id-codes.
    #[error(
        "the template `{template_id}` is id-coded (`{example_node_id}`), and mapping paths are \
         written with at-codes"
    )]
    IdCodedTemplate {
        /// The template that was refused.
        template_id: String,
        /// One id-code the template carries, as evidence.
        example_node_id: String,
    },
    /// A node of the built Web Template carries occurrences no RM multiplicity
    /// can hold.
    #[error("the node at `{aql_path}` has occurrences {min:?}..{max}")]
    MalformedOccurrences {
        /// The `aqlPath` of the offending node.
        aql_path: String,
        /// The lower bound the Web Template carries.
        min: Option<i32>,
        /// The upper bound the Web Template carries.
        max: i32,
    },
    /// A node of the built Web Template carries an `aqlPath` the openEHR path
    /// grammar refuses.
    #[error("the node `aqlPath` `{aql_path}` is not an openEHR path")]
    MalformedAqlPath {
        /// The offending `aqlPath`.
        aql_path: String,
        /// Why the BASE path parser refused it.
        #[source]
        source: openehr_rm::v1_2::paths::PathError,
    },
    /// The mapping path carries a positional predicate (`items[2]`).
    ///
    /// A position selects an instance, never a node, and the engines carry
    /// instance selection as structured occurrences into a build or a read;
    /// a position inside a path would be dropped silently, so it is refused.
    #[error(
        "the mapping path `{path}` carries a positional predicate; occurrences are structured, never written inside a path"
    )]
    PositionalPredicate {
        /// The offending path.
        path: String,
    },
    /// The mapping path could not be resolved against its anchor.
    #[error("the mapping path `{path}` does not resolve against its anchor")]
    Anchor {
        /// The mapping path that was refused.
        path: String,
        /// Why it does not resolve.
        #[source]
        source: PathResolutionError,
    },
    /// No node of the template is at the path.
    #[error("the template `{template_id}` has no node at `{path}`")]
    UnknownPath {
        /// The template that was searched.
        template_id: String,
        /// The path that matched no node.
        path: String,
        /// The `aqlPath` of the deepest node the path does reach, when there
        /// is one.
        nearest_ancestor: Option<String>,
    },
    /// Two or more nodes of the template share the `aqlPath`.
    #[error("the template `{template_id}` has {count} nodes at `{path}`")]
    AmbiguousPath {
        /// The template that was searched.
        template_id: String,
        /// The path that matched more than one node.
        path: String,
        /// How many nodes carry it.
        count: usize,
    },
    /// The node belongs to another template.
    #[error("the template `{template_id}` has no node `{flat_id}`")]
    UnknownNode {
        /// The template that was searched.
        template_id: String,
        /// The flat id that names no node of it.
        flat_id: String,
    },
    /// The child is not under the parent, so no relative path exists.
    #[error("`{child}` is not under `{parent}`")]
    NotADescendant {
        /// The `aqlPath` of the node the path would be relative to.
        parent: String,
        /// The `aqlPath` of the node the path would lead to.
        child: String,
    },
    /// The occurrence list does not match the repeating nodes on the way to
    /// the target.
    #[error("the node `{flat_id}` needs {expected} occurrence indices and was given {given}")]
    OccurrenceCount {
        /// The flat id of the target node.
        flat_id: String,
        /// How many repeating nodes lie on the way to it.
        expected: usize,
        /// How many indices the caller supplied.
        given: usize,
    },
    /// The composition builder refused the path and value pairs.
    #[error("the values do not build a composition of `{template_id}`")]
    CompositionBuild {
        /// The template the composition was built against.
        template_id: String,
        /// Why the builder refused them.
        #[source]
        source: Box<FlatError>,
    },
    /// The built composition does not validate against its template.
    #[error("the composition does not validate against `{template_id}`: {}", messages.join("; "))]
    InvalidComposition {
        /// The template the composition was validated against.
        template_id: String,
        /// One rendered message per violation.
        messages: Vec<String>,
    },
    /// The flattener refused a canonical composition.
    #[error("the composition built against `{template_id}` does not flatten")]
    CompositionFlatten {
        /// The template the composition was built against.
        template_id: String,
        /// Why the flattener refused it.
        #[source]
        source: Box<FlatError>,
    },
    /// The composition was built against another template.
    #[error("the composition names the template `{found}`, not `{expected}`")]
    TemplateMismatch {
        /// The template the index carries.
        expected: String,
        /// The template the composition names.
        found: String,
    },
}

impl PathError {
    /// Returns the code this refusal reports.
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        match *self {
            Self::TemplateParse { .. } | Self::TemplateBuild { .. } => {
                DiagnosticCode::TemplateBuild
            }
            Self::IdCodedTemplate { .. } => DiagnosticCode::IdCodedTemplate,
            Self::MalformedOccurrences { .. }
            | Self::MalformedAqlPath { .. }
            | Self::AmbiguousPath { .. } => DiagnosticCode::MalformedTemplate,
            Self::Anchor { .. } => DiagnosticCode::PathAboveAnchorRoot,
            Self::PositionalPredicate { .. } => DiagnosticCode::PositionalPredicateInPath,
            Self::UnknownPath { .. }
            | Self::UnknownNode { .. }
            | Self::NotADescendant { .. }
            | Self::OccurrenceCount { .. } => DiagnosticCode::UnknownTemplatePath,
            Self::CompositionBuild { .. }
            | Self::CompositionFlatten { .. }
            | Self::InvalidComposition { .. }
            | Self::TemplateMismatch { .. } => DiagnosticCode::InvalidComposition,
        }
    }

    /// Renders this refusal as a diagnostic about `file`.
    #[must_use]
    pub fn to_diagnostic(&self, file: impl Into<std::path::PathBuf>) -> Diagnostic {
        Diagnostic::error(file, self.code(), self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::Generation;
    use super::is_id_code;

    #[test]
    fn an_id_code_is_the_leader_followed_by_a_digit() {
        assert!(is_id_code("id1"));
        assert!(is_id_code("id2.1"));
        assert!(!is_id_code("at0001"));
        assert!(!is_id_code("ac0001"));
        assert!(!is_id_code("identifier"));
        assert!(!is_id_code("openEHR-EHR-EVALUATION.ferrobridge_note.v1"));
        assert!(!is_id_code(""));
    }

    #[test]
    fn a_generation_renders_its_route_name() {
        assert_eq!(Generation::Adl14.to_string(), "adl1.4");
        assert_eq!(Generation::Adl2.to_string(), "adl2");
    }
}
