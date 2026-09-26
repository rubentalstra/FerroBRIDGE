// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals of a run.

use core::fmt;

use openehr_mapping_core::composition::PositionError;
use openehr_mapping_core::template::PathError;
use openehr_rm::v1_2::paths::EhrUriError;

use crate::engine::condition::ConditionError;
use crate::engine::family::FamilyError;
use crate::engine::fhir::FhirError;
use crate::engine::lens::LensError;
use crate::engine::rm::RmError;
use crate::engine::seam::ReferenceError;
use crate::resolve::program::hierarchy::Create;

use crate::tree::error::ReadError;
use crate::tree::error::WriteError;

use crate::engine::traverse::functions::MappingCodeError;

/// Why a run refused.
///
/// Every variant names the mapping it refused in, so a diagnostic points at
/// the file the mapping author edits.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The program's own condition does not admit the input.
    #[error("the context {context} does not apply to this input")]
    NotApplicable {
        /// The context mapping the program compiled from.
        context: String,
    },
    /// A `manual` path writes a `$context` member and the run carries no call
    /// context.
    #[error("{mapping} writes `$context.{member}`, and this run carries no call context")]
    ContextMember {
        /// The dotted name of the manual entry.
        mapping: String,
        /// The `$context` member the path names.
        member: String,
    },
    /// A condition could not be decided.
    #[error("the condition of {mapping} could not be decided")]
    Condition {
        /// The mapping the condition belongs to.
        mapping: String,
        /// The refusal the evaluator returned.
        #[source]
        source: Box<ConditionError>,
    },
    /// The input side could not be read.
    #[error("{mapping} could not read {expression}")]
    Read {
        /// The mapping being run.
        mapping: String,
        /// The expression that refused.
        expression: String,
        /// The refusal the path model returned.
        #[source]
        source: Box<ReadError>,
    },
    /// The output side could not be written.
    #[error("{mapping} could not write {element}")]
    Write {
        /// The mapping being run.
        mapping: String,
        /// The element that refused.
        element: String,
        /// The refusal the path model returned.
        #[source]
        source: Box<WriteError>,
    },
    /// The data-type cell refused the value.
    #[error("{mapping} could not convert {element}")]
    Cell {
        /// The mapping being run.
        mapping: String,
        /// The element the cell refused.
        element: String,
        /// The refusal the cell returned.
        #[source]
        source: Box<LensError>,
    },
    /// An openEHR value could not be read or written.
    #[error("{mapping} could not carry the openEHR value at {node}")]
    Rm {
        /// The mapping being run.
        mapping: String,
        /// The node the value belongs to.
        node: String,
        /// The refusal the value model returned.
        #[source]
        source: Box<RmError>,
    },
    /// A FHIR element could not be read or written.
    #[error("{mapping} could not carry the FHIR element {element}")]
    Fhir {
        /// The mapping being run.
        mapping: String,
        /// The element the value belongs to.
        element: String,
        /// The refusal the element model returned.
        #[source]
        source: Box<FhirError>,
    },
    /// The template could not answer for a node.
    #[error("{mapping} could not reach {node} in the template")]
    Template {
        /// The mapping being run.
        mapping: String,
        /// The node the mapping names.
        node: String,
        /// The refusal the index returned.
        #[source]
        source: Box<PathError>,
    },
    /// An instance index has no openEHR position.
    #[error("{mapping} produced an instance index with no openEHR position")]
    Position {
        /// The mapping being run.
        mapping: String,
        /// The refusal the position model returned.
        #[source]
        source: PositionError,
    },
    /// Neither the mapping nor the element table says which element to write.
    #[error("{mapping} names no FHIR data type for {element}, and the table fixes none")]
    UnknownElement {
        /// The mapping being run.
        mapping: String,
        /// The element the mapping writes.
        element: String,
    },
    /// The mapping names an openEHR path below the template's deepest node
    /// that no FLAT part of the node's class carries.
    ///
    /// A value is written through the Simplified Formats attribute tables of
    /// its class, so an attribute those tables do not name would be merged
    /// into the value and lost on the way to the wire.
    #[error("{mapping} names {tail} below {node}, and no FLAT part of its class carries it")]
    UnsupportedTail {
        /// The mapping being run.
        mapping: String,
        /// The node the path resolved to.
        node: String,
        /// The reference-model attributes below it.
        tail: String,
    },
    /// A required child produced no value.
    ///
    /// "The mapping should fail if the child is not provided"
    /// (`engine/Fail.adoc`).
    #[error("{mapping} produced no value for {node}, which the template requires")]
    MissingRequired {
        /// The mapping that produced nothing.
        mapping: String,
        /// The node the template requires.
        node: String,
    },
    /// A slot chain reaches a model mapping it already entered.
    #[error("the slot chain {chain} reaches {model} a second time")]
    SlotCycle {
        /// The chain as the traversal entered it.
        chain: String,
        /// The model the chain reaches again.
        model: String,
    },
    /// A registered function did not produce a value.
    #[error("{mapping} could not run the function {code}")]
    MappingCode {
        /// The mapping being run.
        mapping: String,
        /// The function the mapping names.
        code: String,
        /// The refusal the registry returned.
        #[source]
        source: Box<MappingCodeError>,
    },
    /// A reference could not be resolved, or a created resource took no id.
    #[error("{mapping} could not resolve its reference")]
    Reference {
        /// The mapping being run.
        mapping: String,
        /// The refusal the seam returned.
        #[source]
        source: Box<ReferenceError>,
    },
    /// A reference chain reaches a resource it already entered.
    ///
    /// "To prevent circular dependencies in FHIR, the mapping engine should
    /// either stop at a given level of references or keep track of which ones
    /// are already resolved" (`engine/references.adoc`).
    #[error("{mapping} reaches {reference} a second time in one reference chain")]
    ReferenceCycle {
        /// The mapping being run.
        mapping: String,
        /// The reference the chain reaches again.
        reference: String,
    },
    /// A `link` target that no `DV_EHR_URI` can carry.
    ///
    /// `LINK.target` is a `DV_EHR_URI`, "a `DV_URI` which has the scheme name
    /// 'ehr'"
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/data_types.html#_dv_ehr_uri_class>).
    #[error("{mapping} links to {target}, which is no ehr: URI")]
    LinkTarget {
        /// The mapping being run.
        mapping: String,
        /// The reference the FHIR side carries.
        target: String,
        /// Why the `ehr:` URI parser of `openehr-rm` refused it.
        #[source]
        source: EhrUriError,
    },
    /// A reference-model family the node carries does not decode.
    #[error("{mapping} could not read the family its node carries")]
    Family {
        /// The mapping being run.
        mapping: String,
        /// The refusal of the typed read.
        #[source]
        source: Box<FamilyError>,
    },
    /// A composition attribute the FLAT builder sets through the `ctx/`
    /// vocabulary carries a part that vocabulary has no key for.
    ///
    /// Simplified Formats master06 §Composer and §Language and Territory set
    /// `COMPOSITION.composer`, `language` and `territory` from `ctx/` keys,
    /// and the builder takes no `|raw` value there, so the part would be lost.
    #[error("{mapping} writes {node}, whose value the FLAT context keys do not carry whole")]
    ContextAttribute {
        /// The mapping, or the node when no mapping is known.
        mapping: String,
        /// The `aqlPath` of the node.
        node: String,
    },
    /// A `link` whose FHIR side is no reference, which is the linked
    /// composition the specification leaves to a second context run.
    ///
    /// "If a context mapping for this `meta.url` is found, it is executed and
    /// saved as a separate composition"
    /// (`types-of-mappings/concept-type/concept-mappings.adoc`, §Linked
    /// mappings), and one run produces one composition.
    #[error("{mapping} links a separate composition, which one run does not produce")]
    LinkedComposition {
        /// The mapping being run.
        mapping: String,
    },
    /// A `hierarchy.split` the template or the direction cannot carry.
    #[error("the split of {model} cannot run: {reason}")]
    Split {
        /// The model mapping the hierarchy belongs to.
        model: String,
        /// What the split names that cannot be created.
        reason: SplitRefusal,
    },
    /// A reference the path model left for the engine to resolve.
    #[error("{mapping} reads through a reference at {expression}, which the engine resolves")]
    DeferredReference {
        /// The mapping being run.
        mapping: String,
        /// The expression that reached the reference.
        expression: String,
    },
    /// The composition did not build.
    #[error("the values the run produced do not build a composition")]
    Build {
        /// The refusal the builder returned.
        #[source]
        source: Box<PathError>,
    },
}

/// Why a `hierarchy.split` cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitRefusal {
    /// The element to create is a template node that does not repeat, so no
    /// second instance of it exists.
    NotRepeating {
        /// The `aqlPath` of the node.
        node: String,
    },
    /// The side names an element the other side creates.
    ///
    /// `resource` is created on the FHIR side and `event` and `archetype` on
    /// the openEHR side (`types-of-mappings/concept-type/HierarchyMappings.adoc`,
    /// §split).
    WrongSide {
        /// The element the side names.
        create: Create,
    },
    /// A FHIR split names a path, and a resource is created at the root.
    ///
    /// "This is left empty if the element is the archetype or resource
    /// itself" (`HierarchyMappings.adoc`, §Hierarchy and unique values).
    ResourcePath,
    /// The side names no element to create.
    NoCreate,
    /// The hierarchy names no `with` path on the side the split iterates.
    NoWith,
}

impl fmt::Display for SplitRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotRepeating { ref node } => {
                write!(f, "{node} does not repeat, so it takes no second instance")
            }
            Self::WrongSide { create } => {
                write!(f, "`{create}` is created on the other side")
            }
            Self::ResourcePath => {
                f.write_str("a resource is created at the root, and the split names a path")
            }
            Self::NoCreate => f.write_str("the split names no element to create"),
            Self::NoWith => f.write_str("the hierarchy names no path to iterate"),
        }
    }
}
