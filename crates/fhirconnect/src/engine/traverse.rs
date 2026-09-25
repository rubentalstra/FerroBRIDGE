// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The interpreter over a compiled program, one traversal for both
//! directions.
//!
//! The walk is top-down in program order. For each mapping it evaluates the
//! input-side conditions, reads the input occurrences, runs the data-type cell
//! the two classes fix, and writes the result where the recurrence rules put
//! it. A `followedBy` child runs once per occurrence its parent bound, which
//! is what pairs two repeating lists against each other
//! (`recurrence/PopulatingAnEntry.adoc`).
//!
//! Where a mapping writes is decided by one rule, applied on whichever side is
//! the output: the axes the parent bound keep their instance, and every
//! repeating element below them takes a fresh instance per input occurrence,
//! because "FHIRconnect always appends to a `0..n` path if it's not iterated"
//! (`recurrence/PopulatingAnEntry.adoc` §Wrong mapping). An output with no
//! repeating element below the parent holds one value, so a second write
//! overwrites the first (`recurrence/Overwriting.adoc`) and a `0..n` input
//! leaves only its last occurrence, with the rest declared lost
//! (`recurrence/TransformingList.adoc`).

use core::fmt;
use std::collections::BTreeMap;

use fhir_types::codec::Object;
use fhir_types::codec::Value;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::composition::FlatIndex;
use openehr_mapping_core::composition::NodeValue;
use openehr_mapping_core::composition::PositionError;
use openehr_mapping_core::composition::RmPosition;
use openehr_mapping_core::index::AqlPath;
use openehr_mapping_core::index::FlatId;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::PathError;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_rm::v1_2::model;
use openehr_rm::v1_2::paths::EhrUri;
use openehr_rm::v1_2::paths::EhrUriError;
use openehr_rm::v1_2::paths::item_at_path;
use openehr_rm::v1_2::support::terminology::openehr_terminology_group_identifiers::OpenehrTerminologyGroupIdentifiersData;

use crate::engine::cell;
use crate::engine::condition;
use crate::engine::condition::ConditionError;
use crate::engine::condition::Verdict;
use crate::engine::context::CallContext;
use crate::engine::family;
use crate::engine::family::FamilyError;
use crate::engine::fhir::FhirError;
use crate::engine::fhir::FhirKind;
use crate::engine::fhir::FhirValue;
use crate::engine::lens::LensError;
use crate::engine::origin::Origin;
use crate::engine::origin::UNKNOWN_SOURCE;
use crate::engine::outcome::Outcome;
use crate::engine::outcome::SkipReason;
use crate::engine::outcome::Warning;
use crate::engine::recurrence::Cardinality;
use crate::engine::recurrence::Placement;
use crate::engine::rm;
use crate::engine::rm::Carried;
use crate::engine::rm::RmError;
use crate::engine::rm::RmValue;
use crate::engine::seam::IdentityRequest;
use crate::engine::seam::ReferenceError;
use crate::engine::seam::Seams;
use crate::model::ast::Direction;
use crate::resolve::derive;
use crate::resolve::program::Attachment;
use crate::resolve::program::Create;
use crate::resolve::program::Derived;
use crate::resolve::program::FhirTarget;
use crate::resolve::program::Hierarchy;
use crate::resolve::program::Manual;
use crate::resolve::program::ManualValue;
use crate::resolve::program::Mapping;
use crate::resolve::program::Method;
use crate::resolve::program::OpenehrTarget;
use crate::resolve::program::Preprocessor;
use crate::resolve::program::Program;
use crate::resolve::program::ResourceType;
use crate::resolve::program::Split;
use crate::resolve::program::Target;
use crate::tree::Occurrence;

use crate::tree::element::Move;
use crate::tree::element::Table;
use crate::tree::error::ReadError;
use crate::tree::error::WriteError;
use crate::tree::read::Selected;
use crate::tree::read::read;
use crate::tree::write::write;

/// How many instances of one repeating node a read probes for.
///
/// The composition carries no instance count of its own, so the read stops at
/// the first absent instance and this is the ceiling on that probe. No
/// specification governs this: our own design.
const INSTANCE_CEILING: u32 = 1024;

/// The `LINK.type` a `link` mapping that names none writes.
///
/// `LINK.type` is mandatory
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_link_class>)
/// and the `link` key leaves it optional. No specification governs the
/// value: our own design.
const LINK_TYPE: &str = "reference";

/// The `external_ref.id.scheme` of a participant a FHIR reference names.
///
/// The id is the FHIR literal reference, so the scheme names FHIR. No
/// specification governs the value: our own design.
const PARTICIPATION_SCHEME: &str = "FHIR";

/// The registry a `mappingCode` resolves against.
///
/// The specification leaves the functions to the engine
/// (`types-of-mappings/concept-type/concept-mappings.adoc`), so the registry
/// is a seam rather than a fixed list, and it ships empty.
pub trait MappingFunctions: fmt::Debug {
    /// Runs the function `code` names over one openEHR value.
    ///
    /// # Errors
    ///
    /// Returns [`MappingCodeError`] when the registry does not hold `code` or
    /// the function refuses its input.
    fn to_fhir(&self, code: &str, source: Option<&RmValue>) -> Result<FhirValue, MappingCodeError>;

    /// Runs the function `code` names over one FHIR element.
    ///
    /// # Errors
    ///
    /// Returns [`MappingCodeError`] when the registry does not hold `code` or
    /// the function refuses its input.
    fn to_openehr(&self, code: &str, view: Option<&FhirValue>)
    -> Result<RmValue, MappingCodeError>;
}

/// The registry this milestone ships.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoMappingFunctions;

impl MappingFunctions for NoMappingFunctions {
    fn to_fhir(
        &self,
        code: &str,
        _source: Option<&RmValue>,
    ) -> Result<FhirValue, MappingCodeError> {
        Err(MappingCodeError::Unknown {
            code: String::from(code),
        })
    }

    fn to_openehr(
        &self,
        code: &str,
        _view: Option<&FhirValue>,
    ) -> Result<RmValue, MappingCodeError> {
        Err(MappingCodeError::Unknown {
            code: String::from(code),
        })
    }
}

/// Why a registered function did not produce a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MappingCodeError {
    /// The registry holds no function of that name.
    #[error("no registered function is named {code}")]
    Unknown {
        /// The name the mapping wrote.
        code: String,
    },
    /// The function refused its input.
    #[error("the function {code} refused its input: {reason}")]
    Refused {
        /// The name the mapping wrote.
        code: String,
        /// What the function refused.
        reason: String,
    },
}

/// The composition fields the engine fills when no mapping does.
///
/// `engine/defaults-for-fields.adoc` puts the composer and the context start
/// time on the engine and the rest on "the project performing the mapping", so
/// only those two carry a value here unless the caller sets more. Every field
/// the engine fills is recorded as [`Warning::Defaulted`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    composer: String,
    start_time: String,
    setting: Setting,
    language: Option<String>,
    territory: Option<String>,
    origin: Option<Origin>,
}

impl Defaults {
    /// Creates the defaults, with `now` as the context start time.
    ///
    /// "For the `context_start_time`, the default mapping time can be used",
    /// and the composer is defaulted "with a value such as `FHIRconnect`
    /// party" (`engine/defaults-for-fields.adoc`). This crate reads no clock,
    /// so the caller passes the time.
    #[must_use]
    pub fn at(now: impl Into<String>) -> Self {
        Self {
            composer: String::from("FHIRconnect"),
            start_time: now.into(),
            setting: Setting {
                code: String::from("238"),
                value: String::from("other care"),
            },
            language: None,
            territory: None,
            origin: None,
        }
    }

    /// Returns these defaults with `name` as the composer.
    #[must_use]
    pub fn with_composer(mut self, name: impl Into<String>) -> Self {
        self.composer = name.into();
        self
    }

    /// Returns these defaults with the `EVENT_CONTEXT.setting` the project
    /// performing the mapping defines.
    ///
    /// `code` and `value` are one concept of the openEHR terminology's
    /// `setting` group, which "should be defined by the project performing the
    /// mapping" and "can be defaulted to one of the valid values"
    /// (`engine/defaults-for-fields.adoc`, which links the group in
    /// `openehr_terminology.xml`). The default is `238` "other care".
    #[must_use]
    pub fn with_setting(mut self, code: impl Into<String>, value: impl Into<String>) -> Self {
        self.setting = Setting {
            code: code.into(),
            value: value.into(),
        };
        self
    }

    /// Returns these defaults with the composition language `code`.
    ///
    /// The code set is ISO 639-1
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html#_composition_class>).
    #[must_use]
    pub fn with_language(mut self, code: impl Into<String>) -> Self {
        self.language = Some(code.into());
        self
    }

    /// Returns these defaults with the composition territory `code`.
    ///
    /// The code set is ISO 3166-1
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html#_composition_class>).
    #[must_use]
    pub fn with_territory(mut self, code: impl Into<String>) -> Self {
        self.territory = Some(code.into());
        self
    }

    /// Returns these defaults with `origin` recorded in the composition's
    /// `FEEDER_AUDIT`.
    ///
    /// The system and the source resource travel as `originating_system_audit`
    /// and `originating_system_item_ids`, and every field the engine defaults
    /// travels as one `feeder_system_item_ids` entry in the order it was
    /// defaulted, so a reader of the stored composition sees what the bridge
    /// supplied
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_feeder_audit_class>).
    /// Which attribute carries which fact is our own design: no specification
    /// governs the allocation.
    #[must_use]
    pub fn with_origin(mut self, origin: Origin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Returns the origin the run records, when the caller set one.
    #[must_use]
    pub const fn origin(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }
}

/// One concept of the openEHR terminology's `setting` group.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Setting {
    code: String,
    value: String,
}

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

/// The occurrence a mapping is bound to, on both sides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Binding {
    fhir: Occurrence,
    openehr: Vec<RmPosition>,
}

/// One openEHR value the run produced.
#[derive(Debug, Clone, PartialEq)]
struct Written {
    flat_id: FlatId,
    positions: Vec<RmPosition>,
    value: Held,
}

/// What the run holds at one place of the composition.
#[derive(Debug, Clone, PartialEq)]
enum Held {
    /// A whole value a cell produced.
    Value(RmValue),
    /// A value written one tail attribute at a time, as canonical JSON, read
    /// as its class when the run builds the composition.
    Partial(serde_json::Map<String, serde_json::Value>),
}

/// The value at the end of an openEHR target, the node's own or its tail's.
#[derive(Debug, Clone, PartialEq)]
enum Leaf {
    /// A data value a cell reads.
    Value(Box<RmValue>),
    /// A scalar attribute, as its text.
    Scalar(String),
}

/// Runs `program` over a FHIR document, producing a composition.
///
/// `seams` carries what the run calls out to: the `mappingCode` registry and
/// the source a `reference` mapping fetches the referenced resource from.
/// `context` carries the per-call values a `manual` path reads through
/// `$context`; pass [`CallContext::new`] when the caller supplied none.
///
/// # Errors
///
/// Returns [`EngineError`] for any element the program cannot map, and for a
/// set of values the template does not admit as a composition.
pub fn to_openehr<T: Table + ?Sized>(
    program: &Program,
    table: &T,
    index: &WebTemplateIndex,
    document: &Value,
    seams: &Seams<'_>,
    defaults: &Defaults,
    context: &CallContext,
) -> Result<Outcome<CanonicalComposition>, EngineError> {
    let mut run = Run::new(
        program,
        table,
        index,
        Direction::FhirToOpenehr,
        *seams,
        context,
        document.clone(),
        None,
    );
    // NOTE: basics/Variables.adoc, `$archetype` is the root as `$resource` is, so one
    // resource maps into one instance of the start archetype and its axes are bound.
    run.pins = program
        .root_axes()
        .iter()
        .map(|axis| (axis.clone(), RmPosition::first()))
        .collect();
    run.admits_context()?;
    run.split_or_run(
        program.mappings(),
        program.hierarchy(),
        program.start().as_str(),
        &Binding::default(),
    )?;
    let defaulted = run.apply_defaults(defaults)?;
    if let Some(origin) = defaults.origin() {
        run.families
            .extend(family::feeder_audit(index.root(), origin, &defaulted));
    }
    let (values, routed) = run.node_values()?;
    let built = index
        .build_composition(&values, &defaults.start_time)
        .map_err(|source| EngineError::Build {
            source: Box::new(source),
        })?;
    run.carried_whole(&built, &routed)?;
    Ok(Outcome::new(built, run.warnings))
}

/// Runs `program` over a composition, producing a FHIR resource.
///
/// `seams` carries what the run calls out to: the `mappingCode` registry and
/// the sink that assigns the id of every resource a `reference` mapping or a
/// `hierarchy.split` creates. Those resources travel as
/// [`Outcome::created`]. `context` carries the per-call values a `manual`
/// path reads through `$context`; pass [`CallContext::new`] when the caller
/// supplied none.
///
/// # Errors
///
/// Returns [`EngineError::NotApplicable`] when the program's own
/// `openehrCondition` does not hold over the composition, and [`EngineError`]
/// for any element the program cannot map.
pub fn to_fhir<T: Table + ?Sized>(
    program: &Program,
    table: &T,
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    seams: &Seams<'_>,
    context: &CallContext,
) -> Result<Outcome<Value>, EngineError> {
    let mut run = Run::new(
        program,
        table,
        index,
        Direction::OpenehrToFhir,
        *seams,
        context,
        empty_resource(program.resource().as_str()),
        Some(composition),
    );
    run.admits_context()?;
    run.split_or_run(
        program.mappings(),
        program.hierarchy(),
        program.start().as_str(),
        &Binding::default(),
    )?;
    let created = core::mem::take(&mut run.created);
    Ok(Outcome::new(run.fhir, run.warnings).with_created(created))
}

/// Returns a resource of `resource_type` that holds nothing else yet.
fn empty_resource(resource_type: &str) -> Value {
    let mut resource = Object::new();
    resource.insert(
        String::from("resourceType"),
        Value::String(String::from(resource_type)),
    );
    Value::Object(resource)
}

/// One run of one program, in one direction.
struct Run<'a, T: Table + ?Sized> {
    program: &'a Program,
    table: &'a T,
    index: &'a WebTemplateIndex,
    direction: Direction,
    seams: Seams<'a>,
    context: &'a CallContext,
    warnings: Vec<Warning>,
    /// The FHIR document the mappings read or write: the run's own resource,
    /// or the referenced or created one a nested run swapped in.
    fhir: Value,
    composition: Option<&'a CanonicalComposition>,
    written: Vec<Written>,
    families: Vec<NodeValue>,
    /// The defaults the run writes as `ctx/` keys.
    context_keys: Vec<NodeValue>,
    /// The values the built composition must carry at the nodes a `ctx/`
    /// default sets, where the key holds less than the value.
    expected: Vec<Routed>,
    counters: BTreeMap<(String, String), usize>,
    chain: Vec<String>,
    /// The references the current reference chain entered, outermost first.
    references: Vec<String>,
    /// The resources the run created beside its own.
    created: Vec<Value>,
    /// The instance a `hierarchy.split` fixes for each openEHR axis it pins.
    pins: Vec<(FlatId, RmPosition)>,
    /// The FHIR element a split out of FHIR fixes, as its repeating axes and
    /// its occurrence.
    fhir_pin: Option<(Vec<String>, Occurrence)>,
}

impl<'a, T: Table + ?Sized> Run<'a, T> {
    /// Creates a run with nothing written yet.
    #[expect(
        clippy::too_many_arguments,
        reason = "a run is assembled once per entry point from exactly the inputs the two entry points take"
    )]
    fn new(
        program: &'a Program,
        table: &'a T,
        index: &'a WebTemplateIndex,
        direction: Direction,
        seams: Seams<'a>,
        context: &'a CallContext,
        fhir: Value,
        composition: Option<&'a CanonicalComposition>,
    ) -> Self {
        Self {
            program,
            table,
            index,
            direction,
            seams,
            context,
            warnings: Vec::new(),
            fhir,
            composition,
            written: Vec::new(),
            families: Vec::new(),
            context_keys: Vec::new(),
            expected: Vec::new(),
            counters: BTreeMap::new(),
            chain: Vec::new(),
            references: Vec::new(),
            created: Vec::new(),
            pins: Vec::new(),
            fhir_pin: None,
        }
    }

    /// Refuses an input the program's own condition does not admit.
    ///
    /// The context's condition is the one the input side carries, the same
    /// rule a mapping's condition follows (`basics/Conditions.adoc`).
    fn admits_context(&self) -> Result<(), EngineError> {
        let gate = match self.direction {
            Direction::FhirToOpenehr => self.program.fhir_condition(),
            Direction::OpenehrToFhir => self.program.openehr_condition(),
        };
        let Some(gate) = gate.filter(|gate| condition::runs(gate, self.direction)) else {
            return Ok(());
        };
        let admits = match self.direction {
            Direction::FhirToOpenehr => condition::evaluate(self.table, &self.fhir, gate, false)
                .map(|verdict| verdict.admits_any())
                .map_err(|source| EngineError::Condition {
                    mapping: String::from(self.program.context().as_str()),
                    source: Box::new(source),
                })?,
            Direction::OpenehrToFhir => self.context_holds(gate)?,
        };
        if admits {
            return Ok(());
        }
        Err(EngineError::NotApplicable {
            context: String::from(self.program.context().as_str()),
        })
    }

    /// Returns whether the context's `openehrCondition` holds over the input.
    fn context_holds(
        &self,
        gate: &crate::resolve::program::Condition,
    ) -> Result<bool, EngineError> {
        self.openehr_holds(self.program.context().as_str(), gate, &[])
    }

    /// Runs a list of mappings under one binding, in program order.
    fn mappings(&mut self, mappings: &[Mapping], parent: &Binding) -> Result<(), EngineError> {
        for mapping in mappings {
            self.mapping(mapping, parent)?;
        }
        Ok(())
    }

    /// Runs one mapping under one binding.
    ///
    /// Answers [`Ran::Gated`] when the unidirectional marker or the input
    /// side's condition kept the mapping from running.
    fn mapping(&mut self, mapping: &Mapping, parent: &Binding) -> Result<Ran, EngineError> {
        if let Some(only) = mapping.direction()
            && only != self.direction
        {
            self.warnings.push(Warning::Skipped {
                mapping: String::from(mapping.name()),
                reason: SkipReason::Unidirectional,
            });
            return Ok(Ran::Gated);
        }
        let Some(admitted) = self.admits(mapping, parent)? else {
            return Ok(Ran::Gated);
        };
        let bindings = self.apply(mapping, parent, &admitted)?;
        for binding in &bindings {
            self.children(mapping, binding)?;
        }
        Ok(Ran::Ran)
    }

    /// Returns the input occurrences the conditions admit, `None` for a
    /// mapping a gate closed.
    ///
    /// A condition that filters out every occurrence the input carries closes
    /// the gate as a false one does: the mapping's input is there, and the
    /// condition says it is none this mapping maps.
    fn admits(
        &self,
        mapping: &Mapping,
        parent: &Binding,
    ) -> Result<Option<Vec<Occurrence>>, EngineError> {
        let inputs = self.inputs(mapping, parent)?;
        let Some(gate) = self.input_condition(mapping) else {
            return Ok(Some(inputs));
        };
        match self.direction {
            Direction::FhirToOpenehr => {
                // NOTE: Conditions.adoc §targetRoot, a condition whose target is the
                // `with` element filters that element's occurrences; the compiler
                // decided the attachment over the anchored paths.
                let attached = gate.attachment() == Attachment::Element;
                let verdict = condition::evaluate(self.table, &self.fhir, gate, attached).map_err(
                    |source| EngineError::Condition {
                        mapping: String::from(mapping.name()),
                        source: Box::new(source),
                    },
                )?;
                match verdict {
                    Verdict::Gate(false) => Ok(None),
                    Verdict::Gate(true) => Ok(Some(inputs)),
                    Verdict::Filter(admitted) => {
                        let present = !inputs.is_empty();
                        let kept: Vec<Occurrence> = inputs
                            .into_iter()
                            .filter(|occurrence| admitted.contains(occurrence))
                            .collect();
                        Ok((!kept.is_empty() || !present).then_some(kept))
                    }
                }
            }
            Direction::OpenehrToFhir => {
                let present = !inputs.is_empty();
                let mut admitted = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let positions = Self::positions_at(mapping, &input)?;
                    if self.openehr_holds(mapping.name(), gate, &positions)? {
                        admitted.push(input);
                    }
                }
                Ok((!admitted.is_empty() || !present).then_some(admitted))
            }
        }
    }

    /// Returns whether an `openehrCondition` holds at one instance.
    ///
    /// The condition is evaluated once per instance of the input, so a
    /// `targetRoot` that is the `with` path filters the instances and one
    /// pointing elsewhere answers the same for all of them, which is the plain
    /// true or false `Conditions.adoc` §targetRoot asks for.
    fn openehr_holds(
        &self,
        name: &str,
        gate: &crate::resolve::program::Condition,
        instance: &[RmPosition],
    ) -> Result<bool, EngineError> {
        let Some(composition) = self.composition else {
            return Ok(true);
        };
        let mut found = condition::Attributes::default();
        for attribute in gate.attributes() {
            let Target::Openehr(ref target) = *attribute else {
                return Err(EngineError::Condition {
                    mapping: String::from(name),
                    source: Box::new(ConditionError::WrongSide),
                });
            };
            let depth = target.occurrences().len();
            let mut positions: Vec<RmPosition> = instance.iter().take(depth).copied().collect();
            positions.resize(depth, RmPosition::first());
            let read = self
                .index
                .read(composition, target.node(), &positions)
                .map_err(|source| EngineError::Template {
                    mapping: String::from(name),
                    node: String::from(target.node().aql_path().as_str()),
                    source: Box::new(source),
                })?;
            let Some(value) = read else {
                continue;
            };
            let Some(at_tail) = attribute_at(&value, target) else {
                continue;
            };
            found.present = found.present.saturating_add(1);
            found.types.push(String::from(target.node().rm_type()));
            if let Some(text) = scalar(at_tail) {
                found.values.push(text);
            }
        }
        Ok(condition::decide(gate, &found))
    }

    /// Returns the positions one input occurrence names, as it was read.
    fn positions_at(mapping: &Mapping, input: &Occurrence) -> Result<Vec<RmPosition>, EngineError> {
        rm_positions(mapping.name(), input.indices())
    }

    /// Returns the condition the direction evaluates, if the mapping carries
    /// one.
    fn input_condition<'mapping>(
        &self,
        mapping: &'mapping Mapping,
    ) -> Option<&'mapping crate::resolve::program::Condition> {
        match self.direction {
            Direction::FhirToOpenehr => mapping.fhir_condition(),
            Direction::OpenehrToFhir => mapping.openehr_condition(),
        }
        .filter(|gate| condition::runs(gate, self.direction))
    }

    /// Returns the occurrences of the input side, under the parent binding.
    fn inputs(&self, mapping: &Mapping, parent: &Binding) -> Result<Vec<Occurrence>, EngineError> {
        match self.direction {
            Direction::FhirToOpenehr => {
                let Some(input) = mapping.fhir() else {
                    return Ok(vec![parent.fhir.clone()]);
                };
                let matches =
                    read(self.table, &self.fhir, input.expression()).map_err(|source| {
                        EngineError::Read {
                            mapping: String::from(mapping.name()),
                            expression: String::from(input.expression().as_str()),
                            source: Box::new(source),
                        }
                    })?;
                let pinned = self
                    .fhir_pin
                    .as_ref()
                    .filter(|&(axes, _)| fhir_axes(input).starts_with(axes))
                    .map(|(_, occurrence)| occurrence);
                let mut occurrences = Vec::new();
                for matched in matches {
                    if !matched
                        .occurrence()
                        .indices()
                        .starts_with(parent.fhir.indices())
                    {
                        continue;
                    }
                    if pinned.is_some_and(|pin| {
                        !matched.occurrence().indices().starts_with(pin.indices())
                    }) {
                        continue;
                    }
                    if let Selected::Deferred(_) = *matched.selected() {
                        return Err(EngineError::DeferredReference {
                            mapping: String::from(mapping.name()),
                            expression: String::from(input.expression().as_str()),
                        });
                    }
                    occurrences.push(matched.occurrence().clone());
                }
                Ok(occurrences)
            }
            Direction::OpenehrToFhir => {
                // NOTE: no specification governs this: our own design, a
                // mapping with no openEHR side reads the instance its parent
                // bound, so its one input is the parent's openEHR occurrence.
                let Some(input) = mapping.openehr() else {
                    return Ok(vec![openehr_occurrence(mapping.name(), &parent.openehr)?]);
                };
                let instances = self.instances(mapping.name(), input, parent)?;
                let mut occurrences = Vec::with_capacity(instances.len());
                for positions in instances {
                    occurrences.push(openehr_occurrence(mapping.name(), &positions)?);
                }
                Ok(occurrences)
            }
        }
    }

    /// Returns the instances of the input node under the parent binding.
    ///
    /// An axis the parent bound or a split pinned keeps its instance. The
    /// composition carries no instance count, so the probe of the first free
    /// axis stops at the first instance the read does not find.
    fn instances(
        &self,
        name: &str,
        input: &OpenehrTarget,
        parent: &Binding,
    ) -> Result<Vec<Vec<RmPosition>>, EngineError> {
        let axes = input.occurrences();
        let depth = axes.len();
        let bound = parent.openehr.len().min(depth);
        let mut prefix: Vec<RmPosition> = parent.openehr.iter().take(bound).copied().collect();
        while let Some(pinned) = axes.get(prefix.len()).and_then(|axis| self.pin(axis)) {
            prefix.push(pinned);
        }
        if prefix.len() >= depth {
            prefix.truncate(depth);
            // NOTE: no specification governs this: our own design, a node the
            // composition does not hold is no input occurrence, so nothing below it runs.
            if self.node_value(name, input, &prefix)?.is_none() {
                return Ok(Vec::new());
            }
            return Ok(vec![prefix]);
        }
        let mut found = Vec::new();
        let mut instance = 1u32;
        while instance <= INSTANCE_CEILING {
            let mut positions = prefix.clone();
            positions.push(
                RmPosition::new(instance).map_err(|source| EngineError::Position {
                    mapping: String::from(name),
                    source,
                })?,
            );
            for axis in axes.iter().skip(positions.len()) {
                positions.push(self.pin(axis).unwrap_or_else(RmPosition::first));
            }
            if self.node_value(name, input, &positions)?.is_none() {
                break;
            }
            found.push(positions);
            instance = instance.saturating_add(1);
        }
        Ok(found)
    }

    /// Returns the canonical JSON one instance of the target's node holds.
    fn node_value(
        &self,
        name: &str,
        input: &OpenehrTarget,
        positions: &[RmPosition],
    ) -> Result<Option<serde_json::Value>, EngineError> {
        let Some(composition) = self.composition else {
            return Ok(None);
        };
        let node = input.node();
        self.index
            .read(composition, node, positions)
            .map_err(|source| EngineError::Template {
                mapping: String::from(name),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            })
    }

    /// Returns the value one instance of the input holds at the end of its
    /// path, below the node when the path names a tail.
    ///
    /// The tail is walked by attribute name over the node's canonical JSON,
    /// and the leaf is read as the class the resolver recorded for it, so the
    /// tail's class selects the cell. A tail no FLAT part of the node's class
    /// carries is refused here as it is on the write side.
    fn value_at(
        &self,
        mapping: &Mapping,
        input: &OpenehrTarget,
        positions: &[RmPosition],
    ) -> Result<Option<Leaf>, EngineError> {
        let Some(value) = self.node_value(mapping.name(), input, positions)? else {
            return Ok(None);
        };
        let node = input.node();
        if !input.tail().segments.is_empty() {
            let segments = tail_segments(input);
            let (_class, carried) = rm::carried(node.rm_type(), &segments)
                .ok_or_else(|| unsupported_tail(mapping, input))?;
            let Some(found) = attribute_at(&value, input) else {
                return Ok(None);
            };
            if !matches!(carried, Carried::Value | Carried::Family) {
                return Ok(scalar(found).map(Leaf::Scalar));
            }
            let leaf = input
                .leaf_class()
                .ok_or_else(|| unsupported_tail(mapping, input))?;
            return RmValue::from_canonical(leaf, node.aql_path().as_str(), found)
                .map(|value| Some(Leaf::Value(Box::new(value))))
                .map_err(|source| EngineError::Rm {
                    mapping: String::from(mapping.name()),
                    node: String::from(node.aql_path().as_str()),
                    source: Box::new(source),
                });
        }
        RmValue::from_canonical(node.rm_type(), node.aql_path().as_str(), &value)
            .map(|value| Some(Leaf::Value(Box::new(value))))
            .map_err(|source| EngineError::Rm {
                mapping: String::from(mapping.name()),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            })
    }

    /// Applies one mapping's method to every admitted input occurrence.
    ///
    /// Returns the binding each occurrence produced, which is what a
    /// `followedBy` child runs under.
    fn apply(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        match *mapping.method() {
            Method::Value => self.values(mapping, parent, inputs),
            Method::Slot {
                ref model,
                ref preprocessors,
                ref mappings,
            } => {
                let name = String::from(model.as_str());
                if self.chain.contains(&name) {
                    return Err(EngineError::SlotCycle {
                        chain: self.chain.join(" -> "),
                        model: name,
                    });
                }
                let bindings = self.bindings(mapping, parent, inputs)?;
                let mut admitted = Vec::with_capacity(bindings.len());
                for binding in &bindings {
                    if self.slot_admits(mapping, preprocessors, binding)? {
                        admitted.push(binding);
                    } else {
                        self.warnings.push(Warning::Skipped {
                            mapping: String::from(mapping.name()),
                            reason: SkipReason::PreprocessorGate {
                                model: name.clone(),
                            },
                        });
                    }
                }
                // NOTE: HierarchyMappings.adoc, the hierarchy lives in the
                // preprocessor of the file it belongs to, so a slotted file
                // splits the slotted mappings under each slot binding.
                let hierarchy = preprocessors
                    .iter()
                    .find(|preprocessor| preprocessor.model() == model)
                    .and_then(Preprocessor::hierarchy);
                self.chain.push(name);
                for binding in admitted {
                    self.split_or_run(mappings, hierarchy, model.as_str(), binding)?;
                }
                self.chain.pop();
                Ok(bindings)
            }
            Method::Programmed { ref code } => self.programmed(mapping, parent, inputs, code),
            Method::Reference {
                ref resource,
                ref mappings,
            } => self.reference(mapping, parent, inputs, resource, mappings),
            Method::Link {
                ref meaning,
                ref link_type,
            } => {
                let meaning = meaning.as_deref().unwrap_or(mapping.name());
                let link_type = link_type.as_deref().unwrap_or(LINK_TYPE);
                self.link(mapping, parent, inputs, meaning, link_type)
            }
            Method::Participation { ref function } => {
                self.participation(mapping, parent, inputs, function)
            }
        }
    }

    /// Runs a list of mappings, split by the hierarchy of the file they came
    /// from when it splits the direction's output.
    ///
    /// `split.fhir` creates FHIR resources, so it runs going out of openEHR,
    /// and `split.openehr` creates openEHR elements, so it runs going into
    /// openEHR (`types-of-mappings/concept-type/HierarchyMappings.adoc`,
    /// §split).
    fn split_or_run(
        &mut self,
        mappings: &[Mapping],
        hierarchy: Option<&Hierarchy>,
        model: &str,
        parent: &Binding,
    ) -> Result<(), EngineError> {
        let Some(hierarchy) = hierarchy else {
            return self.mappings(mappings, parent);
        };
        match self.direction {
            Direction::OpenehrToFhir => match hierarchy.split_fhir() {
                Some(split) => self.split_resources(mappings, hierarchy, split, model, parent),
                None => self.mappings(mappings, parent),
            },
            Direction::FhirToOpenehr => match hierarchy.split_openehr() {
                Some(split) => self.split_elements(mappings, hierarchy, split, model, parent),
                None => self.mappings(mappings, parent),
            },
        }
    }

    /// Creates one FHIR resource per occurrence of the split's openEHR path
    /// and distinct `unique` tuple.
    ///
    /// "For each occurrence of `$archetype/data[at0001]/events[at0002]`, the
    /// `split` is executed", and "one could imagine this process as cloning
    /// the composition with one event each"
    /// (`types-of-mappings/concept-type/HierarchyMappings.adoc`, §split): the
    /// whole mapping set runs once per group with the split node pinned to
    /// the group's occurrence, so content outside the node reaches every
    /// resource. The first group fills the run's own resource and every
    /// further one is a created resource with an id from the identity sink.
    /// A split with no `unique` key makes every occurrence its own group.
    fn split_resources(
        &mut self,
        mappings: &[Mapping],
        hierarchy: &Hierarchy,
        split: &Split,
        model: &str,
        parent: &Binding,
    ) -> Result<(), EngineError> {
        let refuse = |reason: SplitRefusal| EngineError::Split {
            model: String::from(model),
            reason,
        };
        match split.create() {
            Some(Create::Resource) => {}
            Some(create) => return Err(refuse(SplitRefusal::WrongSide { create })),
            None => return Err(refuse(SplitRefusal::NoCreate)),
        }
        if split.path().is_some() {
            return Err(refuse(SplitRefusal::ResourcePath));
        }
        let with = hierarchy
            .openehr()
            .ok_or_else(|| refuse(SplitRefusal::NoWith))?;
        let mut groups: Vec<(Vec<String>, Vec<Vec<RmPosition>>)> = Vec::new();
        for positions in self.instances(model, with, parent)? {
            let tuple = self.unique_openehr(model, split.unique(), &positions)?;
            match groups
                .iter_mut()
                .find(|group| !split.unique().is_empty() && group.0 == tuple)
            {
                Some(group) => group.1.push(positions),
                None => groups.push((tuple, vec![positions])),
            }
        }
        if groups.is_empty() {
            return self.mappings(mappings, parent);
        }
        let base = self.fhir.clone();
        let start = self.warnings.len();
        for (index, (tuple, members)) in groups.into_iter().enumerate() {
            if index == 0 {
                self.run_pinned(mappings, parent, with, &members)?;
                continue;
            }
            let outer = core::mem::replace(&mut self.fhir, base.clone());
            let counters = core::mem::take(&mut self.counters);
            let ran = self.run_pinned(mappings, parent, with, &members);
            self.counters = counters;
            let mut created = core::mem::replace(&mut self.fhir, outer);
            ran?;
            let occurrence = members
                .first()
                .map(|positions| positions.iter().map(|position| position.get()).collect())
                .unwrap_or_default();
            let request = self
                .identity_request(self.program.resource().as_str(), model)
                .with_occurrence(occurrence)
                .with_unique(tuple);
            self.identify(model, &mut created, &request)?;
            self.created.push(created);
        }
        self.settle_split_warnings(start);
        Ok(())
    }

    /// Runs a mapping set once per member, with the split node pinned to it.
    fn run_pinned(
        &mut self,
        mappings: &[Mapping],
        parent: &Binding,
        with: &OpenehrTarget,
        members: &[Vec<RmPosition>],
    ) -> Result<(), EngineError> {
        for positions in members {
            let pins: Vec<(FlatId, RmPosition)> = with
                .occurrences()
                .iter()
                .cloned()
                .zip(positions.iter().copied())
                .collect();
            let held = core::mem::replace(&mut self.pins, pins);
            let ran = self.mappings(mappings, parent);
            self.pins = held;
            ran?;
        }
        Ok(())
    }

    /// Creates one openEHR element per occurrence of the split's FHIR path
    /// and distinct `unique` tuple.
    ///
    /// "For each `dosage` with a different `route` and/or `timing.event`,
    /// create a new `EVENT` in openEHR" (`HierarchyMappings.adoc`, §Hierarchy
    /// and unique values): each group pins the FHIR path to its occurrences
    /// and the created node to a fresh instance, so every mapping under the
    /// node writes into the group's own element.
    fn split_elements(
        &mut self,
        mappings: &[Mapping],
        hierarchy: &Hierarchy,
        split: &Split,
        model: &str,
        parent: &Binding,
    ) -> Result<(), EngineError> {
        let refuse = |reason: SplitRefusal| EngineError::Split {
            model: String::from(model),
            reason,
        };
        match split.create() {
            Some(Create::Event | Create::Archetype) => {}
            Some(create) => return Err(refuse(SplitRefusal::WrongSide { create })),
            None => return Err(refuse(SplitRefusal::NoCreate)),
        }
        let with = hierarchy
            .fhir()
            .ok_or_else(|| refuse(SplitRefusal::NoWith))?;
        let node = match split.path() {
            Some(Target::Openehr(target)) => target.as_ref(),
            Some(Target::Fhir(_)) | None => hierarchy
                .openehr()
                .ok_or_else(|| refuse(SplitRefusal::NoWith))?,
        };
        let axes = node.occurrences();
        if axes.last() != Some(node.node().flat_id()) {
            return Err(refuse(SplitRefusal::NotRepeating {
                node: String::from(node.node().aql_path().as_str()),
            }));
        }
        let matches = read(self.table, &self.fhir, with.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(model),
                expression: String::from(with.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        let mut groups: Vec<(Vec<String>, Vec<Occurrence>)> = Vec::new();
        for matched in matches {
            if !matched
                .occurrence()
                .indices()
                .starts_with(parent.fhir.indices())
            {
                continue;
            }
            let occurrence = matched.occurrence().clone();
            let tuple = self.unique_fhir(model, split.unique(), &occurrence)?;
            match groups
                .iter_mut()
                .find(|group| !split.unique().is_empty() && group.0 == tuple)
            {
                Some(group) => group.1.push(occurrence),
                None => groups.push((tuple, vec![occurrence])),
            }
        }
        if groups.is_empty() {
            return self.mappings(mappings, parent);
        }
        let with_axes = fhir_axes(with);
        let start = self.warnings.len();
        for (index, (_tuple, members)) in groups.into_iter().enumerate() {
            let instance = u32::try_from(index)
                .ok()
                .and_then(|index| RmPosition::try_from(FlatIndex::new(index)).ok())
                .ok_or(EngineError::Position {
                    mapping: String::from(model),
                    source: PositionError::Overflow,
                })?;
            let mut pins: Vec<(FlatId, RmPosition)> = Vec::with_capacity(axes.len());
            for (depth, axis) in axes.iter().enumerate() {
                let position = if depth.saturating_add(1) == axes.len() {
                    instance
                } else {
                    parent
                        .openehr
                        .get(depth)
                        .copied()
                        .unwrap_or_else(RmPosition::first)
                };
                pins.push((axis.clone(), position));
            }
            for occurrence in members {
                let held = core::mem::replace(&mut self.pins, pins.clone());
                let held_fhir = self.fhir_pin.replace((with_axes.clone(), occurrence));
                let ran = self.mappings(mappings, parent);
                self.pins = held;
                self.fhir_pin = held_fhir;
                ran?;
            }
        }
        self.settle_split_warnings(start);
        Ok(())
    }

    /// Keeps one of each warning a split's groups declared alike when the
    /// warning does not depend on the group.
    ///
    /// A mapping skipped for its `unidirectional` marker is skipped for every
    /// group the same way, so the split declares that loss once; a warning
    /// that depends on the group's data (a dropped occurrence, a one-way row,
    /// an unresolved reference) stays once per group. No specification governs
    /// this: our own design.
    fn settle_split_warnings(&mut self, start: usize) {
        settle(&mut self.warnings, start);
    }

    /// Returns the `unique` tuple one openEHR occurrence carries.
    ///
    /// "The path in the `unique:` key relates to the `with:` method"
    /// (`HierarchyMappings.adoc`), so each value is read at the occurrence's
    /// own instance, and an absent value is the empty text.
    fn unique_openehr(
        &self,
        model: &str,
        unique: &[Target],
        positions: &[RmPosition],
    ) -> Result<Vec<String>, EngineError> {
        let Some(composition) = self.composition else {
            return Ok(Vec::new());
        };
        let mut tuple = Vec::with_capacity(unique.len());
        for target in unique {
            let Target::Openehr(ref target) = *target else {
                tuple.push(String::new());
                continue;
            };
            let depth = target.occurrences().len();
            let mut at: Vec<RmPosition> = positions.iter().take(depth).copied().collect();
            at.resize(depth, RmPosition::first());
            let read = self
                .index
                .read(composition, target.node(), &at)
                .map_err(|source| EngineError::Template {
                    mapping: String::from(model),
                    node: String::from(target.node().aql_path().as_str()),
                    source: Box::new(source),
                })?;
            // NOTE: no specification governs this: our own design, a data value
            // compares by the text of its `value`, which is what a reader of
            // the element sees, and any other structure by its JSON.
            let text = read
                .as_ref()
                .and_then(|value| attribute_at(value, target))
                .map(|value| {
                    scalar(value)
                        .or_else(|| value.get("value").and_then(scalar))
                        .unwrap_or_else(|| value.to_string())
                })
                .unwrap_or_default();
            tuple.push(text);
        }
        Ok(tuple)
    }

    /// Returns the `unique` tuple one FHIR occurrence carries.
    fn unique_fhir(
        &self,
        model: &str,
        unique: &[Target],
        occurrence: &Occurrence,
    ) -> Result<Vec<String>, EngineError> {
        let mut tuple = Vec::with_capacity(unique.len());
        for target in unique {
            let Target::Fhir(ref target) = *target else {
                tuple.push(String::new());
                continue;
            };
            let matches = read(self.table, &self.fhir, target.expression()).map_err(|source| {
                EngineError::Read {
                    mapping: String::from(model),
                    expression: String::from(target.expression().as_str()),
                    source: Box::new(source),
                }
            })?;
            let mut texts = Vec::new();
            for matched in &matches {
                if !matched
                    .occurrence()
                    .indices()
                    .starts_with(occurrence.indices())
                {
                    continue;
                }
                if let Some(value) = matched.value() {
                    texts.push(lexical(value));
                }
            }
            tuple.push(texts.join("|"));
        }
        Ok(tuple)
    }

    /// Returns the openEHR pin of one axis, when a split pinned it.
    fn pin(&self, axis: &FlatId) -> Option<RmPosition> {
        self.pin_named(axis.as_str())
    }

    /// Returns the openEHR pin of one axis, by its flat id.
    ///
    /// Only an openEHR axis takes a pin; a FHIR output axis is a FHIR path,
    /// which no flat id equals.
    fn pin_named(&self, axis: &str) -> Option<RmPosition> {
        self.pins
            .iter()
            .find(|pinned| pinned.0.as_str() == axis)
            .map(|pinned| pinned.1)
    }

    /// Returns the identity request of a resource this run creates.
    fn identity_request(&self, resource_type: &str, mapping: &str) -> IdentityRequest {
        let mut request =
            IdentityRequest::new(resource_type, self.program.resource().as_str(), mapping);
        if let Some(id) = self.fhir.get("id").and_then(Value::as_str) {
            request = request.with_parent_id(id);
        }
        if let Some(uid) = self
            .composition
            .and_then(|composition| composition.value().get("uid"))
            .and_then(|uid| uid.get("value"))
            .and_then(serde_json::Value::as_str)
        {
            request = request.with_composition(uid);
        }
        request
    }

    /// Asks the identity sink for a created resource's id and sets it.
    fn identify(
        &self,
        mapping: &str,
        created: &mut Value,
        request: &IdentityRequest,
    ) -> Result<String, EngineError> {
        let id = self
            .seams
            .identities()
            .identify(request)
            .map_err(|source| EngineError::Reference {
                mapping: String::from(mapping),
                source: Box::new(source),
            })?;
        if let Value::Object(ref mut object) = *created {
            object.insert(String::from("id"), Value::String(id.clone()));
        }
        Ok(id)
    }

    /// Runs a `reference` mapping.
    ///
    /// "It allows us to initialize a new resource in FHIR (or the other way
    /// around) and reference it inside the resource we are currently mapping"
    /// (`types-of-mappings/concept-type/Reference.adoc`). Going into openEHR
    /// the referenced resource is found in the current document's
    /// `contained`, then through the reference source, and its mappings run
    /// over it; going out of openEHR a new resource is created, its mappings
    /// write it, and the reference to it is written in the current one.
    fn reference(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        resource: &ResourceType,
        mappings: &[Mapping],
    ) -> Result<Vec<Binding>, EngineError> {
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut bindings = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let binding = Binding {
                        fhir: input.clone(),
                        openehr: parent.openehr.clone(),
                    };
                    self.enter_reference(mapping, input, resource, mappings, &binding)?;
                    bindings.push(binding);
                }
                Ok(bindings)
            }
            Direction::OpenehrToFhir => {
                let bindings = self.bindings(mapping, parent, inputs)?;
                for binding in &bindings {
                    self.create_reference(mapping, resource, mappings, binding)?;
                }
                Ok(bindings)
            }
        }
    }

    /// Resolves one reference occurrence and runs the mappings over it.
    fn enter_reference(
        &mut self,
        mapping: &Mapping,
        input: &Occurrence,
        resource: &ResourceType,
        mappings: &[Mapping],
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let Some(literal) = self.reference_at(mapping, input)? else {
            self.warnings.push(Warning::Skipped {
                mapping: String::from(mapping.name()),
                reason: SkipReason::EmptyReference,
            });
            return Ok(());
        };
        if self.references.contains(&literal) {
            return Err(EngineError::ReferenceCycle {
                mapping: String::from(mapping.name()),
                reference: literal,
            });
        }
        let Some(fetched) = self.resolve_reference(mapping, &literal, resource)? else {
            self.warnings.push(Warning::Skipped {
                mapping: String::from(mapping.name()),
                reason: SkipReason::UnresolvedReference { reference: literal },
            });
            return Ok(());
        };
        let outer = core::mem::replace(&mut self.fhir, fetched);
        let held_pin = self.fhir_pin.take();
        self.references.push(literal);
        let inner = Binding {
            fhir: Occurrence::default(),
            openehr: binding.openehr.clone(),
        };
        let ran = self.mappings(mappings, &inner);
        self.references.pop();
        self.fhir_pin = held_pin;
        self.fhir = outer;
        ran
    }

    /// Returns the resource a literal reference points at, checked against
    /// the type the mapping names.
    ///
    /// A `#id` reference names a resource in the current document's
    /// `contained` (<https://hl7.org/fhir/R4/references.html#contained>); any
    /// other goes to the reference source.
    fn resolve_reference(
        &self,
        mapping: &Mapping,
        literal: &str,
        resource: &ResourceType,
    ) -> Result<Option<Value>, EngineError> {
        let refuse = |source: ReferenceError| EngineError::Reference {
            mapping: String::from(mapping.name()),
            source: Box::new(source),
        };
        let found = if let Some(local) = literal.strip_prefix('#') {
            self.fhir
                .get("contained")
                .and_then(Value::as_array)
                .and_then(|contained| {
                    contained
                        .iter()
                        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(local))
                })
                .cloned()
        } else {
            self.seams
                .references()
                .fetch(literal, resource)
                .map_err(refuse)?
        };
        let Some(found) = found else {
            return Ok(None);
        };
        let kind = found
            .get("resourceType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind != resource.as_str() {
            return Err(refuse(ReferenceError::WrongType {
                reference: String::from(literal),
                expected: String::from(resource.as_str()),
                found: String::from(kind),
            }));
        }
        Ok(Some(found))
    }

    /// Returns the literal reference the mapping's FHIR side holds at one
    /// occurrence, `None` when it holds none.
    ///
    /// The side names either a `Reference` or its `reference` string, as the
    /// specification's example writes it (`Reference.adoc`).
    fn reference_at(
        &self,
        mapping: &Mapping,
        input: &Occurrence,
    ) -> Result<Option<String>, EngineError> {
        let Some(value) = self.fhir_value_at(mapping, input)? else {
            return Ok(None);
        };
        Ok(match value {
            Value::String(text) => Some(text),
            Value::Object(ref object) => object
                .get("reference")
                .and_then(Value::as_str)
                .map(String::from),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
        })
    }

    /// Returns the raw FHIR value the mapping's FHIR side holds at one
    /// occurrence.
    fn fhir_value_at(
        &self,
        mapping: &Mapping,
        input: &Occurrence,
    ) -> Result<Option<Value>, EngineError> {
        let Some(target) = mapping.fhir() else {
            return Ok(None);
        };
        let matches = read(self.table, &self.fhir, target.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(mapping.name()),
                expression: String::from(target.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        Ok(matches
            .iter()
            .find(|matched| matched.occurrence() == input)
            .and_then(crate::tree::read::Match::value)
            .cloned())
    }

    /// Creates one referenced resource, and references it from the current
    /// one.
    fn create_reference(
        &mut self,
        mapping: &Mapping,
        resource: &ResourceType,
        mappings: &[Mapping],
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let outer = core::mem::replace(&mut self.fhir, empty_resource(resource.as_str()));
        let counters = core::mem::take(&mut self.counters);
        let inner = Binding {
            fhir: Occurrence::default(),
            openehr: binding.openehr.clone(),
        };
        let ran = self.mappings(mappings, &inner);
        self.counters = counters;
        let mut created = core::mem::replace(&mut self.fhir, outer);
        ran?;
        // NOTE: no specification governs this: our own design, a resource its
        // mappings wrote nothing into carries no content, so no resource and no
        // reference to it is created.
        if created.as_object().is_none_or(|object| object.len() <= 1) {
            return Ok(());
        }
        let request = self
            .identity_request(resource.as_str(), mapping.name())
            .with_occurrence(
                binding
                    .openehr
                    .iter()
                    .map(|position| position.get())
                    .collect(),
            );
        let id = self.identify(mapping.name(), &mut created, &request)?;
        self.created.push(created);
        self.write_reference(
            mapping,
            binding,
            &format!("{}/{id}", resource.as_str()),
            None,
        )
    }

    /// Writes one literal reference at the mapping's FHIR side.
    ///
    /// A side that names a `Reference` takes the element, one that names its
    /// `reference` string takes the string.
    fn write_reference(
        &mut self,
        mapping: &Mapping,
        binding: &Binding,
        literal: &str,
        display: Option<&str>,
    ) -> Result<(), EngineError> {
        let Some(target) = mapping.fhir() else {
            return Ok(());
        };
        let element = String::from(target.resolved().leaf());
        let value = if FhirKind::at(target.resolved().location()) == Some(FhirKind::Reference) {
            let mut object = Object::new();
            object.insert(
                String::from("reference"),
                Value::String(String::from(literal)),
            );
            if let Some(display) = display {
                object.insert(
                    String::from("display"),
                    Value::String(String::from(display)),
                );
            }
            Value::Object(object)
        } else {
            Value::String(String::from(literal))
        };
        write(
            self.table,
            &mut self.fhir,
            target.expression(),
            &binding.fhir,
            value,
        )
        .map_err(|source| EngineError::Write {
            mapping: String::from(mapping.name()),
            element,
            source: Box::new(source),
        })
    }

    /// Runs a `link` mapping.
    ///
    /// The openEHR side names the `links` of a `LOCATABLE`, and each link is
    /// written as one `_link:i` family on the node (Simplified Formats, the
    /// `LINK` table). The FHIR side is the reference the link targets; a side
    /// that is no reference is the linked composition of
    /// `concept-mappings.adoc` §Linked mappings, which one run does not
    /// produce, and is refused.
    fn link(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        meaning: &str,
        link_type: &str,
    ) -> Result<Vec<Binding>, EngineError> {
        let (target, openehr) = Self::family_sides(mapping, "links")?;
        if !matches!(
            FhirKind::at(target.resolved().location()),
            Some(FhirKind::Reference | FhirKind::String)
        ) {
            return Err(EngineError::LinkedComposition {
                mapping: String::from(mapping.name()),
            });
        }
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut bindings = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let binding = Binding {
                        fhir: input.clone(),
                        openehr: parent.openehr.clone(),
                    };
                    let Some(literal) = self.reference_at(mapping, input)? else {
                        self.warnings.push(Warning::Skipped {
                            mapping: String::from(mapping.name()),
                            reason: SkipReason::EmptyReference,
                        });
                        bindings.push(binding);
                        continue;
                    };
                    {
                        if let Err(source) = literal.parse::<EhrUri>() {
                            return Err(EngineError::LinkTarget {
                                mapping: String::from(mapping.name()),
                                target: literal,
                                source,
                            });
                        }
                        let positions = self.family_positions(openehr, &binding.openehr);
                        let index = self.next_family(openehr, &positions, "_link");
                        self.families.extend(family::link(
                            openehr.node(),
                            &positions,
                            index,
                            &family::LinkParts {
                                meaning,
                                link_type,
                                target: &literal,
                            },
                        ));
                    }
                    bindings.push(binding);
                }
                Ok(bindings)
            }
            Direction::OpenehrToFhir => {
                let positions = self.family_positions(openehr, &parent.openehr);
                let Some(node) = self.node_value(mapping.name(), openehr, &positions)? else {
                    return Ok(Vec::new());
                };
                let targets =
                    family::link_targets(&node, meaning, link_type).map_err(|source| {
                        EngineError::Family {
                            mapping: String::from(mapping.name()),
                            source: Box::new(source),
                        }
                    })?;
                let occurrences =
                    vec![openehr_occurrence(mapping.name(), &positions)?; targets.len()];
                let bindings = self.bindings(mapping, parent, &occurrences)?;
                for (binding, literal) in bindings.iter().zip(targets.iter()) {
                    self.write_reference(mapping, binding, literal, None)?;
                }
                Ok(bindings)
            }
        }
    }

    /// Runs a `participationsFunction` mapping.
    ///
    /// "The function in this method is not something that is
    /// auto-transformable. Therefore, it is added to the `with:` statement"
    /// (`concept-mappings.adoc`, §Participation mappings): the FHIR side is
    /// the `Reference` of the participant, the openEHR side the
    /// `other_participations` of an `ENTRY` or the `participations` of the
    /// `EVENT_CONTEXT` (`family::participation_list`), and the function is
    /// the method's. The performer carries the reference as its
    /// `external_ref` and the reference's `display` as its name; the
    /// allocation is our own design.
    fn participation(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        function: &str,
    ) -> Result<Vec<Binding>, EngineError> {
        let list = mapping
            .openehr()
            .and_then(|openehr| {
                let tail = tail_segments(openehr);
                family::participation_list(openehr.node().rm_type(), &tail)
            })
            .unwrap_or(family::ENTRY_PARTICIPATIONS);
        let (_target, openehr) = Self::family_sides(mapping, list.attribute)?;
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut bindings = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let binding = Binding {
                        fhir: input.clone(),
                        openehr: parent.openehr.clone(),
                    };
                    let value = self.fhir_value_at(mapping, input)?;
                    let literal = value
                        .as_ref()
                        .and_then(|value| value.get("reference"))
                        .and_then(Value::as_str);
                    let display = value
                        .as_ref()
                        .and_then(|value| value.get("display"))
                        .and_then(Value::as_str);
                    let Some(literal) = literal else {
                        self.warnings.push(Warning::Skipped {
                            mapping: String::from(mapping.name()),
                            reason: SkipReason::EmptyReference,
                        });
                        bindings.push(binding);
                        continue;
                    };
                    let namespace = self.reference_type(literal);
                    let positions = self.family_positions(openehr, &binding.openehr);
                    let index = self.next_family(openehr, &positions, list.family);
                    self.families.extend(family::participation(
                        openehr.node(),
                        list,
                        &positions,
                        index,
                        &family::ParticipationParts {
                            function,
                            name: display,
                            id: literal,
                            id_scheme: PARTICIPATION_SCHEME,
                            id_namespace: namespace.unwrap_or(UNKNOWN_SOURCE),
                        },
                    ));
                    bindings.push(binding);
                }
                Ok(bindings)
            }
            Direction::OpenehrToFhir => {
                let positions = self.family_positions(openehr, &parent.openehr);
                let Some(node) = self.node_value(mapping.name(), openehr, &positions)? else {
                    return Ok(Vec::new());
                };
                let found = family::participations(&node, list, function).map_err(|source| {
                    EngineError::Family {
                        mapping: String::from(mapping.name()),
                        source: Box::new(source),
                    }
                })?;
                let occurrences =
                    vec![openehr_occurrence(mapping.name(), &positions)?; found.len()];
                let bindings = self.bindings(mapping, parent, &occurrences)?;
                for (binding, participant) in bindings.iter().zip(found.iter()) {
                    let Some(ref literal) = participant.id else {
                        continue;
                    };
                    self.write_reference(mapping, binding, literal, participant.name.as_deref())?;
                }
                Ok(bindings)
            }
        }
    }

    /// Returns the two sides of a mapping that writes a reference-model
    /// family, refusing an openEHR side whose tail is not that family.
    fn family_sides<'mapping>(
        mapping: &'mapping Mapping,
        attribute: &str,
    ) -> Result<(&'mapping FhirTarget, &'mapping OpenehrTarget), EngineError> {
        let Some(openehr) = mapping.openehr() else {
            return Err(EngineError::LinkedComposition {
                mapping: String::from(mapping.name()),
            });
        };
        if tail_segments(openehr) != [attribute] {
            return Err(unsupported_tail(mapping, openehr));
        }
        let Some(target) = mapping.fhir() else {
            return Err(EngineError::UnknownElement {
                mapping: String::from(mapping.name()),
                element: String::from(attribute),
            });
        };
        Ok((target, openehr))
    }

    /// Returns the positions a family on the target's node is written at.
    fn family_positions(&self, target: &OpenehrTarget, bound: &[RmPosition]) -> Vec<RmPosition> {
        target
            .occurrences()
            .iter()
            .enumerate()
            .map(|(depth, axis)| {
                bound
                    .get(depth)
                    .copied()
                    .or_else(|| self.pin(axis))
                    .unwrap_or_else(RmPosition::first)
            })
            .collect()
    }

    /// Returns the next free index of one family on one node instance.
    fn next_family(
        &mut self,
        target: &OpenehrTarget,
        positions: &[RmPosition],
        family: &str,
    ) -> usize {
        let key = (
            format!("{family}@{}", target.node().flat_id()),
            positions
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<String>>()
                .join("."),
        );
        let next = self.counters.entry(key).or_insert(0);
        let taken = *next;
        *next = next.saturating_add(1);
        taken
    }

    /// Returns the resource type a literal reference names, when it names one
    /// this FHIR version defines.
    ///
    /// A literal reference is `<type>/<id>`, relative or at the end of an
    /// absolute URL, optionally followed by `/_history/<version>`
    /// (<https://hl7.org/fhir/R4/references.html#literal>).
    fn reference_type<'text>(&self, literal: &'text str) -> Option<&'text str> {
        let segments: Vec<&str> = literal.split('/').collect();
        let end = segments
            .iter()
            .rposition(|segment| *segment == "_history")
            .unwrap_or(segments.len());
        let kind = end.checked_sub(2).and_then(|at| segments.get(at))?;
        self.table.is_resource(kind).then_some(*kind)
    }

    /// Returns whether every preprocessor gate of a slotted file admits the
    /// input at `binding`.
    ///
    /// A file's preprocessor condition "defines that the mapping file is only
    /// executed if the given condition is met" (`basics/Conditions.adoc`,
    /// §Conditions in the preprocessor), read on the input side like every
    /// condition, so a closed gate skips the slotted mappings for that
    /// occurrence and the skip is a recorded outcome.
    fn slot_admits(
        &self,
        mapping: &Mapping,
        preprocessors: &[Preprocessor],
        binding: &Binding,
    ) -> Result<bool, EngineError> {
        for preprocessor in preprocessors {
            let gate = match self.direction {
                Direction::FhirToOpenehr => preprocessor.fhir_condition(),
                Direction::OpenehrToFhir => preprocessor.openehr_condition(),
            };
            let Some(gate) = gate.filter(|gate| condition::runs(gate, self.direction)) else {
                continue;
            };
            let holds = match self.direction {
                Direction::FhirToOpenehr => {
                    condition::evaluate(self.table, &self.fhir, gate, false)
                        .map(|verdict| verdict.admits_any())
                        .map_err(|source| EngineError::Condition {
                            mapping: String::from(mapping.name()),
                            source: Box::new(source),
                        })?
                }
                Direction::OpenehrToFhir => {
                    self.openehr_holds(preprocessor.model().as_str(), gate, &binding.openehr)?
                }
            };
            if !holds {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Returns the text a `manual` path writes.
    ///
    /// A `$context` member is a per-call value the caller supplies, so a
    /// member this run does not carry is a refusal rather than an invented
    /// value (`basics/Variables.adoc`, §`$context`).
    fn manual_text<'run>(
        &'run self,
        mapping: &Mapping,
        entry: &Manual,
        value: &'run ManualValue,
    ) -> Result<&'run str, EngineError> {
        match *value {
            ManualValue::Literal(ref text) => Ok(text.as_str()),
            ManualValue::Context(ref member) => {
                self.context
                    .member(member)
                    .ok_or_else(|| EngineError::ContextMember {
                        mapping: format!("{}.{}", mapping.name(), entry.name()),
                        member: member.clone(),
                    })
            }
        }
    }

    /// Runs the data-type cell of a plain `with` mapping, or only binds.
    fn values(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        let bindings = self.bindings(mapping, parent, inputs)?;
        if !mapping.manual().is_empty() {
            for entry in mapping.manual() {
                for binding in &bindings {
                    self.manual(mapping, entry, binding)?;
                }
            }
            return Ok(bindings);
        }
        // NOTE: PopulatingAnEntry.adoc writes `type: NONE` on the parent whose
        // `followedBy` children carry every value, so the parent anchors them
        // and transforms nothing of its own (recorded as a silence in #185).
        if mapping.data_type() == Some(crate::model::ast::DataType::None)
            || matches!(mapping.derived(), Some(Derived::Anchor))
        {
            return Ok(bindings);
        }
        let (Some(_fhir), Some(_openehr)) = (mapping.fhir(), mapping.openehr()) else {
            return Ok(bindings);
        };
        for (input, binding) in inputs.iter().zip(bindings.iter()) {
            self.convert(mapping, input, binding)?;
        }
        Ok(bindings)
    }

    /// Runs the registered function a `mappingCode` names.
    fn programmed(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        code: &str,
    ) -> Result<Vec<Binding>, EngineError> {
        let bindings = self.bindings(mapping, parent, inputs)?;
        let refuse = |source: MappingCodeError| EngineError::MappingCode {
            mapping: String::from(mapping.name()),
            code: String::from(code),
            source: Box::new(source),
        };
        for binding in &bindings {
            match self.direction {
                Direction::FhirToOpenehr => {
                    let produced = self
                        .seams
                        .functions()
                        .to_openehr(code, None)
                        .map_err(refuse)?;
                    self.put_openehr(mapping, binding, produced);
                }
                Direction::OpenehrToFhir => {
                    let produced = self.seams.functions().to_fhir(code, None).map_err(refuse)?;
                    self.put_fhir(mapping, binding, &produced)?;
                }
            }
        }
        Ok(bindings)
    }

    /// Runs one occurrence of a mapping through its data-type cell.
    fn convert(
        &mut self,
        mapping: &Mapping,
        input: &Occurrence,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        match self.direction {
            Direction::FhirToOpenehr => {
                let Some(target) = mapping.fhir() else {
                    return Ok(());
                };
                let Some(openehr) = mapping.openehr() else {
                    return Ok(());
                };
                let Some(view) = self.input_value(mapping, target, openehr, input)? else {
                    return Ok(());
                };
                let node = openehr.node();
                if !openehr.tail().segments.is_empty() {
                    return self.convert_tail(mapping, target, openehr, &view, binding);
                }
                let held = self.held(&binding.openehr, node.flat_id());
                let produced = cell::put(
                    &view,
                    node.rm_type(),
                    held.as_ref(),
                    self.binding_of(node).as_deref(),
                )
                .map_err(|source| EngineError::Cell {
                    mapping: String::from(mapping.name()),
                    element: String::from(target.expression().as_str()),
                    source: Box::new(source),
                })?;
                for taken in &produced.fallbacks {
                    self.warnings.push(Warning::fallback(taken));
                }
                self.put_openehr(mapping, binding, produced.value);
                Ok(())
            }
            Direction::OpenehrToFhir => {
                let Some(target) = mapping.fhir() else {
                    return Ok(());
                };
                let Some(openehr) = mapping.openehr() else {
                    return Ok(());
                };
                let positions = Self::positions_of(mapping, input, openehr)?;
                let Some(source) = self.value_at(mapping, openehr, &positions)? else {
                    return Ok(());
                };
                let (target, kind) = Self::output_target(mapping, target)?;
                let refuse = |source: LensError| EngineError::Cell {
                    mapping: String::from(mapping.name()),
                    element: String::from(target.expression().as_str()),
                    source: Box::new(source),
                };
                let view = match source {
                    Leaf::Value(ref value) => {
                        cell::get(value, kind, self.binding_of(openehr.node()).as_deref())
                            .map_err(refuse)?
                    }
                    // NOTE: a scalar leaf carries text, so only an element that
                    // travels as text takes it; no specification governs this:
                    // our own design.
                    Leaf::Scalar(ref text) => match kind {
                        FhirKind::String | FhirKind::DateTime => {
                            let element = target.resolved().leaf();
                            FhirValue::read(kind, element, &Value::String(text.clone())).map_err(
                                |source| EngineError::Fhir {
                                    mapping: String::from(mapping.name()),
                                    element: String::from(element),
                                    source: Box::new(source),
                                },
                            )?
                        }
                        FhirKind::Coding
                        | FhirKind::CodeableConcept
                        | FhirKind::Period
                        | FhirKind::Reference
                        | FhirKind::Identifier
                        | FhirKind::Quantity => {
                            return Err(refuse(LensError::NoCell {
                                rm_type: String::from(openehr.leaf_class().unwrap_or("String")),
                                kind: kind.as_str(),
                                direction: Direction::OpenehrToFhir,
                            }));
                        }
                    },
                };
                self.put_fhir_at(mapping, target, binding, &view)
            }
        }
    }

    /// Reads the FHIR element one input occurrence names, as the conversion
    /// the compiler derived or the `type` key names.
    fn input_value(
        &self,
        mapping: &Mapping,
        target: &FhirTarget,
        openehr: &OpenehrTarget,
        input: &Occurrence,
    ) -> Result<Option<FhirValue>, EngineError> {
        match mapping.derived() {
            Some(Derived::Choice { read, .. }) => self.choice_at(mapping, read, openehr, input),
            Some(Derived::Declared(declared)) => {
                let kind = Self::kind(mapping, declared.target())?;
                self.element_at(mapping, declared.target(), input, kind)
            }
            Some(Derived::Element(_) | Derived::Anchor) | None => {
                let kind = Self::kind(mapping, target)?;
                self.element_at(mapping, target, input, kind)
            }
        }
    }

    /// Returns the target a mapping writes FHIR through, with the element
    /// kind it writes.
    fn output_target<'mapping>(
        mapping: &'mapping Mapping,
        target: &'mapping FhirTarget,
    ) -> Result<(&'mapping FhirTarget, FhirKind), EngineError> {
        match mapping.derived() {
            Some(Derived::Choice {
                write: Some(written),
                ..
            }) => {
                let kind = FhirKind::of_code(written.code()).ok_or_else(|| {
                    EngineError::UnknownElement {
                        mapping: String::from(mapping.name()),
                        element: String::from(written.target().resolved().leaf()),
                    }
                })?;
                Ok((written.target(), kind))
            }
            Some(Derived::Declared(declared)) => {
                Ok((declared.target(), Self::kind(mapping, declared.target())?))
            }
            Some(Derived::Choice { write: None, .. }) => Err(EngineError::UnknownElement {
                mapping: String::from(mapping.name()),
                element: String::from(target.resolved().leaf()),
            }),
            Some(Derived::Element(_) | Derived::Anchor) | None => {
                Ok((target, Self::kind(mapping, target)?))
            }
        }
    }

    /// Writes one FHIR element into the attribute a tail names below a node.
    ///
    /// The tail's leaf class selects the cell when the leaf is a data value,
    /// and a scalar leaf takes the text of the FHIR element. The attribute is
    /// merged into what the place already holds, so several mappings that
    /// name attributes of one value build that one value, and the whole value
    /// is read as its class when the composition is built.
    fn convert_tail(
        &mut self,
        mapping: &Mapping,
        target: &FhirTarget,
        openehr: &OpenehrTarget,
        view: &FhirValue,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let node = openehr.node();
        let segments = tail_segments(openehr);
        let (class, carried) = rm::carried(node.rm_type(), &segments)
            .ok_or_else(|| unsupported_tail(mapping, openehr))?;
        let refuse_rm = |source: RmError| EngineError::Rm {
            mapping: String::from(mapping.name()),
            node: String::from(node.aql_path().as_str()),
            source: Box::new(source),
        };
        if carried == Carried::Family {
            let leaf = openehr
                .leaf_class()
                .ok_or_else(|| unsupported_tail(mapping, openehr))?;
            let produced = cell::put(view, leaf, None, self.binding_of(node).as_deref()).map_err(
                |source| EngineError::Cell {
                    mapping: String::from(mapping.name()),
                    element: String::from(target.expression().as_str()),
                    source: Box::new(source),
                },
            )?;
            for taken in &produced.fallbacks {
                self.warnings.push(Warning::fallback(taken));
            }
            let RmValue::Party(ref party) = produced.value else {
                return Err(refuse_rm(RmError::UnknownClass {
                    rm_type: String::from(produced.value.rm_type()),
                }));
            };
            self.families.extend(family::provider(
                node,
                &binding.openehr,
                &rm::family_of(&segments),
                party,
            ));
            return Ok(());
        }
        let mut object = self
            .held_object(&binding.openehr, node.flat_id())
            .map_err(refuse_rm)?;
        let written = if carried == Carried::Value {
            let leaf = openehr
                .leaf_class()
                .ok_or_else(|| unsupported_tail(mapping, openehr))?;
            // NOTE: no specification governs this: our own design, a held
            // sub-value still being written attribute by attribute is no whole
            // value yet, so it carries nothing over and the cell starts fresh.
            let held = serde_json::Value::Object(object.clone());
            let existing = item_at_path(&held, openehr.tail()).and_then(|found| {
                RmValue::from_canonical(leaf, node.aql_path().as_str(), found).ok()
            });
            let produced = cell::put(
                view,
                leaf,
                existing.as_ref(),
                self.binding_of(node).as_deref(),
            )
            .map_err(|source| EngineError::Cell {
                mapping: String::from(mapping.name()),
                element: String::from(target.expression().as_str()),
                source: Box::new(source),
            })?;
            for taken in &produced.fallbacks {
                self.warnings.push(Warning::fallback(taken));
            }
            produced.value.to_canonical().map_err(refuse_rm)?
        } else {
            let element = String::from(target.resolved().leaf());
            let text = view.write(&element).map_err(|source| EngineError::Fhir {
                mapping: String::from(mapping.name()),
                element: element.clone(),
                source: Box::new(source),
            })?;
            scalar_json(carried, class, node, &text).map_err(refuse_rm)?
        };
        merge_value(&mut object, &segments, written);
        let kept = object
            .get("_type")
            .and_then(serde_json::Value::as_str)
            .filter(|held| *held == class || model::is_a(held, class))
            .map(String::from);
        object.insert(
            String::from("_type"),
            serde_json::Value::String(kept.unwrap_or_else(|| String::from(class))),
        );
        self.store(Written {
            flat_id: node.flat_id().clone(),
            positions: binding.openehr.clone(),
            value: Held::Partial(object),
        });
        Ok(())
    }

    /// Returns the canonical JSON the run holds at one place, empty when it
    /// holds nothing there.
    fn held_object(
        &self,
        positions: &[RmPosition],
        flat_id: &FlatId,
    ) -> Result<serde_json::Map<String, serde_json::Value>, RmError> {
        let Some(written) = self
            .written
            .iter()
            .find(|written| written.flat_id == *flat_id && written.positions == positions)
        else {
            return Ok(serde_json::Map::new());
        };
        match written.value {
            Held::Partial(ref object) => Ok(object.clone()),
            Held::Value(ref value) => match value.to_canonical()? {
                serde_json::Value::Object(object) => Ok(object),
                _ => Ok(serde_json::Map::new()),
            },
        }
    }

    /// Stores one written value, overwriting what the same place holds.
    fn store(&mut self, written: Written) {
        if let Some(slot) = self
            .written
            .iter_mut()
            .find(|held| held.flat_id == written.flat_id && held.positions == written.positions)
        {
            *slot = written;
        } else {
            self.written.push(written);
        }
    }

    /// Returns the positions one input occurrence names for an openEHR node.
    fn positions_of(
        mapping: &Mapping,
        input: &Occurrence,
        target: &OpenehrTarget,
    ) -> Result<Vec<RmPosition>, EngineError> {
        let depth = target.occurrences().len();
        let taken = input.indices().get(..depth).unwrap_or(input.indices());
        let mut positions = rm_positions(mapping.name(), taken)?;
        positions.resize(depth, RmPosition::first());
        Ok(positions)
    }

    /// Returns which FHIR element a mapping writes.
    ///
    /// The `type` key names it where the mapping carries one, and the pair
    /// the compiler derived otherwise.
    fn kind(mapping: &Mapping, target: &FhirTarget) -> Result<FhirKind, EngineError> {
        let kind = match (mapping.data_type(), mapping.derived()) {
            (Some(data_type), _) => FhirKind::of(data_type),
            (None, Some(Derived::Element(code))) => FhirKind::of_code(code),
            (None, _) => FhirKind::at(target.resolved().location()),
        };
        kind.ok_or_else(|| EngineError::UnknownElement {
            mapping: String::from(mapping.name()),
            element: String::from(target.resolved().leaf()),
        })
    }

    /// Reads the choice element one input occurrence names, as the
    /// alternative the document carries.
    ///
    /// The JSON key of a choice names its type
    /// (<https://hl7.org/fhir/R4/formats.html#choice>), so the instance fixes
    /// the kind, and the node's class decides how a text alternative reads
    /// (`crate::resolve::derive::carried`).
    fn choice_at(
        &self,
        mapping: &Mapping,
        read_at: &FhirTarget,
        openehr: &OpenehrTarget,
        occurrence: &Occurrence,
    ) -> Result<Option<FhirValue>, EngineError> {
        let matches = read(self.table, &self.fhir, read_at.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(mapping.name()),
                expression: String::from(read_at.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        let Some(found) = matches
            .iter()
            .find(|matched| matched.occurrence() == occurrence)
        else {
            return Ok(None);
        };
        let (Some(value), Some((suffix, variant))) = (found.value(), found.alternative()) else {
            return Ok(None);
        };
        let element = read_at.resolved().leaf();
        let class = openehr.leaf_class().unwrap_or(openehr.node().rm_type());
        let kind = derive::carried(class, suffix, variant)
            .and_then(FhirKind::of_code)
            .ok_or_else(|| EngineError::UnknownElement {
                mapping: String::from(mapping.name()),
                element: format!("{element} as {suffix}"),
            })?;
        FhirValue::read(kind, element, value)
            .map(Some)
            .map_err(|source| EngineError::Fhir {
                mapping: String::from(mapping.name()),
                element: String::from(element),
                source: Box::new(source),
            })
    }

    /// Reads the FHIR element one input occurrence names.
    fn element_at(
        &self,
        mapping: &Mapping,
        target: &FhirTarget,
        occurrence: &Occurrence,
        kind: FhirKind,
    ) -> Result<Option<FhirValue>, EngineError> {
        let matches = read(self.table, &self.fhir, target.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(mapping.name()),
                expression: String::from(target.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        let Some(found) = matches
            .iter()
            .find(|matched| matched.occurrence() == occurrence)
            .and_then(crate::tree::read::Match::value)
        else {
            return Ok(None);
        };
        FhirValue::read(kind, target.resolved().leaf(), found)
            .map(Some)
            .map_err(|source| EngineError::Fhir {
                mapping: String::from(mapping.name()),
                element: String::from(target.resolved().leaf()),
                source: Box::new(source),
            })
    }

    /// Returns the terminology the template binds an openEHR node to.
    fn binding_of(&self, node: &ResolvedNode) -> Option<String> {
        self.index
            .bindings(node)
            .ok()
            .and_then(<[openehr_mapping_core::template::Binding]>::first)
            .map(|binding| String::from(binding.system()))
    }

    /// Returns the openEHR value the run already wrote at one place.
    ///
    /// A value still being written attribute by attribute is no whole value
    /// yet, so it holds nothing a cell can carry over.
    fn held(&self, positions: &[RmPosition], flat_id: &FlatId) -> Option<RmValue> {
        self.written
            .iter()
            .find(|written| written.flat_id == *flat_id && written.positions == positions)
            .and_then(|written| match written.value {
                Held::Value(ref value) => Some(value.clone()),
                Held::Partial(_) => None,
            })
    }

    /// Writes one openEHR value, overwriting what the same place holds.
    fn put_openehr(&mut self, mapping: &Mapping, binding: &Binding, value: RmValue) {
        let Some(target) = mapping.openehr() else {
            return;
        };
        self.store(Written {
            flat_id: target.node().flat_id().clone(),
            positions: binding.openehr.clone(),
            value: Held::Value(value),
        });
    }

    /// Writes one FHIR element at the occurrence its binding names.
    fn put_fhir(
        &mut self,
        mapping: &Mapping,
        binding: &Binding,
        view: &FhirValue,
    ) -> Result<(), EngineError> {
        let Some(target) = mapping.fhir() else {
            return Ok(());
        };
        self.put_fhir_at(mapping, target, binding, view)
    }

    /// Writes one FHIR element through `target` at the occurrence its
    /// binding names.
    fn put_fhir_at(
        &mut self,
        mapping: &Mapping,
        target: &FhirTarget,
        binding: &Binding,
        view: &FhirValue,
    ) -> Result<(), EngineError> {
        let element = String::from(target.resolved().leaf());
        let value = view.write(&element).map_err(|source| EngineError::Fhir {
            mapping: String::from(mapping.name()),
            element: element.clone(),
            source: Box::new(source),
        })?;
        write(
            self.table,
            &mut self.fhir,
            target.expression(),
            &binding.fhir,
            value,
        )
        .map_err(|source| EngineError::Write {
            mapping: String::from(mapping.name()),
            element,
            source: Box::new(source),
        })
    }

    /// Writes one manual entry, merging every path it names into one element.
    fn manual(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        if let Some(only) = entry.direction()
            && only != self.direction
        {
            self.warnings.push(Warning::Skipped {
                mapping: format!("{}.{}", mapping.name(), entry.name()),
                reason: SkipReason::Unidirectional,
            });
            return Ok(());
        }
        if !self.manual_admits(mapping, entry, binding)? {
            return Ok(());
        }
        match self.direction {
            Direction::FhirToOpenehr => self.manual_openehr(mapping, entry, binding),
            Direction::OpenehrToFhir => self.manual_fhir(mapping, entry, binding),
        }
    }

    /// Returns whether the input-side conditions admit a manual entry.
    ///
    /// Going out of openEHR the entry's `openehrCondition` is evaluated at the
    /// instance the mapping bound, the rule a mapping's own condition follows
    /// (`basics/Conditions.adoc`, "conditions are always applied on the input
    /// data").
    fn manual_admits(
        &self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<bool, EngineError> {
        let gate = match self.direction {
            Direction::FhirToOpenehr => entry.fhir_condition(),
            Direction::OpenehrToFhir => entry.openehr_condition(),
        };
        let Some(gate) = gate.filter(|gate| condition::runs(gate, self.direction)) else {
            return Ok(true);
        };
        match self.direction {
            Direction::FhirToOpenehr => condition::evaluate(self.table, &self.fhir, gate, false)
                .map(|verdict| verdict.admits_any())
                .map_err(|source| EngineError::Condition {
                    mapping: String::from(mapping.name()),
                    source: Box::new(source),
                }),
            Direction::OpenehrToFhir => self.openehr_holds(mapping.name(), gate, &binding.openehr),
        }
    }

    /// Writes the openEHR paths of one manual entry as one value.
    fn manual_openehr(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let mut merged = serde_json::Map::new();
        let mut node: Option<&OpenehrTarget> = None;
        for path in entry.openehr() {
            let Target::Openehr(ref target) = *path.target() else {
                continue;
            };
            node = Some(target.as_ref());
            let segments: Vec<&str> = target
                .tail()
                .segments
                .iter()
                .map(|segment| segment.attribute.as_str())
                .collect();
            // NOTE: no specification governs this: our own design, `merge` writes a manual
            // value as text, so a tail ending on no string attribute refuses (an empty
            // tail writes the node's `value`).
            let written: &[&str] = if segments.is_empty() {
                &["value"]
            } else {
                &segments
            };
            if rm::carried(target.node().rm_type(), written)
                .is_none_or(|(_, held)| held != Carried::Text)
            {
                return Err(EngineError::UnsupportedTail {
                    mapping: format!("{}.{}", mapping.name(), entry.name()),
                    node: String::from(target.node().aql_path().as_str()),
                    tail: target.tail().to_string(),
                });
            }
            let text = self.manual_text(mapping, entry, path.value())?;
            merge(&mut merged, &segments, text);
        }
        let Some(target) = node else {
            return Ok(());
        };
        let rm_type = target.node().rm_type();
        merged.insert(
            String::from("_type"),
            serde_json::Value::String(String::from(rm_type)),
        );
        let value = RmValue::from_canonical(
            rm_type,
            target.node().aql_path().as_str(),
            &serde_json::Value::Object(merged),
        )
        .map_err(|source| EngineError::Rm {
            mapping: format!("{}.{}", mapping.name(), entry.name()),
            node: String::from(target.node().aql_path().as_str()),
            source: Box::new(source),
        })?;
        self.store(Written {
            flat_id: target.node().flat_id().clone(),
            positions: binding.openehr.clone(),
            value: Held::Value(value),
        });
        Ok(())
    }

    /// Writes the FHIR paths of one manual entry into one element.
    fn manual_fhir(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let mut shared: BTreeMap<String, usize> = BTreeMap::new();
        for path in entry.fhir() {
            let Target::Fhir(ref target) = *path.target() else {
                continue;
            };
            let occurrence = self.entry_occurrence(target, binding, &mut shared);
            let element = String::from(target.resolved().leaf());
            let text = Value::String(String::from(self.manual_text(
                mapping,
                entry,
                path.value(),
            )?));
            write(
                self.table,
                &mut self.fhir,
                target.expression(),
                &occurrence,
                text,
            )
            .map_err(|source| EngineError::Write {
                mapping: format!("{}.{}", mapping.name(), entry.name()),
                element,
                source: Box::new(source),
            })?;
        }
        Ok(())
    }

    /// Returns the occurrence one path of a manual entry writes at.
    ///
    /// "Full FHIR or openEHR elements are created from all given paths"
    /// (`types-of-mappings/concept-type/manual.adoc`), so every path of one
    /// entry that passes the same repeating element takes the same instance
    /// of it.
    fn entry_occurrence(
        &mut self,
        target: &FhirTarget,
        binding: &Binding,
        shared: &mut BTreeMap<String, usize>,
    ) -> Occurrence {
        let axes = fhir_axes(target);
        let mut indices: Vec<usize> = binding.fhir.indices().to_vec();
        for axis in axes.iter().skip(indices.len()) {
            let index = if let Some(&taken) = shared.get(axis) {
                taken
            } else {
                let taken = self.next_index(axis, &indices);
                shared.insert(axis.clone(), taken);
                taken
            };
            indices.push(index);
        }
        Occurrence::new(indices)
    }

    /// Returns the next free instance of one axis under one prefix.
    fn next_index(&mut self, axis: &str, prefix: &[usize]) -> usize {
        let key = (String::from(axis), render(prefix));
        let next = self.counters.entry(key).or_insert(0);
        let taken = *next;
        *next = next.saturating_add(1);
        taken
    }

    /// Returns the binding each input occurrence writes under.
    fn bindings(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        let axes = self.output_axes(mapping);
        let mut bound = parent_depth(parent, self.direction);
        while axes
            .get(bound)
            .is_some_and(|axis| self.pin_named(axis).is_some())
        {
            bound = bound.saturating_add(1);
        }
        let cardinality = Cardinality::of(axes.len() > bound);
        let placement = Placement::decide(inputs.len(), cardinality, 0);
        if placement.lossy() {
            self.warnings.push(Warning::LastOfMany {
                path: self.output_name(mapping),
                dropped: placement.dropped(),
            });
        }
        let mut bindings = Vec::with_capacity(inputs.len());
        for (position, input) in inputs.iter().enumerate() {
            let taken = placement.slots().get(position).copied().flatten();
            bindings.push(self.bind(mapping, parent, input, &axes, taken)?);
        }
        Ok(bindings)
    }

    /// Returns one binding, appending at every axis the parent did not bind.
    fn bind(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        input: &Occurrence,
        axes: &[String],
        taken: Option<usize>,
    ) -> Result<Binding, EngineError> {
        let bound = parent_depth(parent, self.direction);
        let mut indices: Vec<usize> = match self.direction {
            // NOTE: an openEHR position is 1-based and an instance index is
            // 0-based (Simplified Formats §Instance Indexing), so the parent's
            // positions come back through the index the counters speak.
            Direction::FhirToOpenehr => {
                let mut held = Vec::with_capacity(parent.openehr.len());
                for position in &parent.openehr {
                    let index = FlatIndex::from(*position).get();
                    held.push(usize::try_from(index).map_err(|_refused| {
                        EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source: PositionError::Overflow,
                        }
                    })?);
                }
                held
            }
            Direction::OpenehrToFhir => parent.fhir.indices().to_vec(),
        };
        indices.truncate(bound);
        if taken.is_some() {
            let mut fresh = false;
            for axis in axes.iter().skip(bound) {
                let index = if let Some(pinned) = self.pin_named(axis) {
                    usize::try_from(FlatIndex::from(pinned).get()).map_err(|_refused| {
                        EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source: PositionError::Overflow,
                        }
                    })?
                } else if fresh {
                    0
                } else {
                    fresh = true;
                    let key = (axis.clone(), render(&indices));
                    let next = self.counters.entry(key).or_insert(0);
                    let taken = *next;
                    *next = next.saturating_add(1);
                    taken
                };
                indices.push(index);
            }
        }
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut positions = Vec::with_capacity(indices.len());
                for index in &indices {
                    let index =
                        u32::try_from(*index).map_err(|_refused| EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source: PositionError::Overflow,
                        })?;
                    positions.push(RmPosition::try_from(FlatIndex::new(index)).map_err(
                        |source| EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source,
                        },
                    )?);
                }
                Ok(Binding {
                    fhir: input.clone(),
                    openehr: positions,
                })
            }
            Direction::OpenehrToFhir => Ok(Binding {
                fhir: Occurrence::new(indices),
                openehr: rm_positions(mapping.name(), input.indices())?,
            }),
        }
    }

    /// Returns the repeating elements of the output side, outermost first.
    fn output_axes(&self, mapping: &Mapping) -> Vec<String> {
        match self.direction {
            Direction::FhirToOpenehr => mapping.openehr().map_or_else(Vec::new, |target| {
                target
                    .occurrences()
                    .iter()
                    .map(|axis| String::from(axis.as_str()))
                    .collect()
            }),
            Direction::OpenehrToFhir => mapping.fhir().map_or_else(Vec::new, fhir_axes),
        }
    }

    /// Returns the name of the output side, for a declared loss.
    fn output_name(&self, mapping: &Mapping) -> String {
        match self.direction {
            Direction::FhirToOpenehr => mapping
                .openehr()
                .map(|target| String::from(target.node().aql_path().as_str()))
                .unwrap_or_default(),
            Direction::OpenehrToFhir => mapping
                .fhir()
                .map(|target| String::from(target.resolved().leaf()))
                .unwrap_or_default(),
        }
    }

    /// Runs the `followedBy` children of one mapping under its binding.
    ///
    /// "If we have a parent node with a `1..1` cardinality and a child node
    /// with a `1..1` cardinality, the mapping should fail if the child is not
    /// provided" (`engine/Fail.adoc`), so a child the template requires and
    /// the input does not carry refuses the unit. A child its own condition
    /// closed was provided and is none the mapping maps, so it is skipped,
    /// and a structural node counts as provided once any value below it is
    /// written.
    fn children(&mut self, mapping: &Mapping, binding: &Binding) -> Result<(), EngineError> {
        for child in mapping.followed_by() {
            let ran = self.mapping(child, binding)?;
            if self.direction != Direction::FhirToOpenehr || ran == Ran::Gated {
                continue;
            }
            let Some(target) = child.openehr() else {
                continue;
            };
            if target.node().min().is_none_or(|min| min < 1) {
                continue;
            }
            if !self.provided(target) {
                return Err(EngineError::MissingRequired {
                    mapping: String::from(child.name()),
                    node: String::from(target.node().aql_path().as_str()),
                });
            }
        }
        Ok(())
    }

    /// Returns whether the run wrote the node a required child names.
    ///
    /// A structural node holds no value of its own, so it is provided once
    /// the run wrote any value at or below it.
    fn provided(&self, target: &OpenehrTarget) -> bool {
        let node = target.node().flat_id();
        if !derive::is_structural(target.node().rm_type()) {
            return self.written.iter().any(|written| written.flat_id == *node);
        }
        let below = format!("{}/", node.as_str());
        self.written
            .iter()
            .any(|written| written.flat_id == *node || written.flat_id.as_str().starts_with(&below))
            || self.families.iter().any(|family| {
                family
                    .flat_id()
                    .is_some_and(|flat_id| flat_id == node || flat_id.as_str().starts_with(&below))
            })
    }

    /// Fills the composition fields no mapping wrote.
    ///
    /// Returns the openEHR path of every field it filled, in the order it
    /// filled them, which is the order of the [`Warning::Defaulted`] entries
    /// it recorded. Which fields take a default is FHIRconnect's
    /// (`engine/defaults-for-fields.adoc`); every default travels as the `ctx/`
    /// key the FLAT builder resolves (Simplified Formats, master06 §Composer,
    /// §time, §setting, §Language and Territory). `ctx/setting` carries the
    /// code and the builder takes the rubric from the openEHR terminology's
    /// [`OpenehrTerminologyGroupIdentifiersData::GROUP_ID_SETTING`] group, so
    /// the project's value is checked against the built composition.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Template`] when the template cannot answer for a
    /// defaulted field.
    fn apply_defaults(&mut self, defaults: &Defaults) -> Result<Vec<String>, EngineError> {
        let mut filled = Vec::new();
        self.default_context(
            &mut filled,
            "/composer",
            "composer_name",
            &defaults.composer,
        )?;
        self.default_context(
            &mut filled,
            "/context/start_time",
            "time",
            &defaults.start_time,
        )?;
        if let Some(node) = self.unwritten("/context/setting")? {
            let setting = defaults.setting.clone();
            self.context_keys.push(NodeValue::context(
                "setting",
                serde_json::Value::String(setting.code.clone()),
            ));
            self.expected.push((
                node.flat_id().clone(),
                RmValue::CodedText(Box::new(DvCodedText {
                    value: setting.value,
                    hyperlink: None,
                    formatting: None,
                    mappings: None,
                    language: None,
                    encoding: None,
                    defining_code: CodePhrase {
                        terminology_id: TerminologyId {
                            value: String::from(
                                OpenehrTerminologyGroupIdentifiersData::TERMINOLOGY_ID_OPENEHR,
                            ),
                        },
                        code_string: setting.code,
                        preferred_term: None,
                    },
                })),
            ));
            self.defaulted(&mut filled, "/context/setting");
        }
        if let Some(ref code) = defaults.language {
            self.default_context(&mut filled, "/language", "language", code)?;
        }
        if let Some(ref code) = defaults.territory {
            self.default_context(&mut filled, "/territory", "territory", code)?;
        }
        Ok(filled)
    }

    /// Writes one default as the `ctx/` key `field`, when the template holds
    /// a node at `path` and nothing wrote it already.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Template`] when the template cannot answer for
    /// the path for any reason other than holding no node at it.
    fn default_context(
        &mut self,
        filled: &mut Vec<String>,
        path: &str,
        field: &str,
        value: &str,
    ) -> Result<(), EngineError> {
        if self.unwritten(path)?.is_some() {
            self.context_keys.push(NodeValue::context(
                field,
                serde_json::Value::String(String::from(value)),
            ));
            self.defaulted(filled, path);
        }
        Ok(())
    }

    /// Records that the engine filled the field at `path`.
    fn defaulted(&mut self, filled: &mut Vec<String>, path: &str) {
        self.warnings.push(Warning::Defaulted {
            field: String::from(path),
        });
        filled.push(String::from(path));
    }

    /// Returns the node at `path` when the template holds one and no mapping
    /// wrote it.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Template`] when the template cannot answer for
    /// the path for any reason other than holding no node at it.
    fn unwritten(&self, path: &str) -> Result<Option<&'a ResolvedNode>, EngineError> {
        let node = match self.index.node(&AqlPath::new(path)) {
            Ok(node) => node,
            // NOTE: engine/defaults-for-fields.adoc, the context "is usually
            // populated", so a template with no node at the path takes no default.
            Err(PathError::UnknownPath { .. }) => return Ok(None),
            Err(source) => {
                return Err(EngineError::Template {
                    mapping: String::from(self.program.context().as_str()),
                    node: String::from(path),
                    source: Box::new(source),
                });
            }
        };
        let written = self
            .written
            .iter()
            .any(|written| written.flat_id == *node.flat_id());
        Ok((!written).then_some(node))
    }

    /// Returns the values the run produced, as the composition builder wants
    /// them, with the values the builder takes through the `ctx/` keys.
    ///
    /// Every value travels whole as its canonical JSON under `|raw`
    /// (Simplified Formats, master04 §Raw canonical JSON), except at a
    /// composition attribute the builder sets from the context vocabulary
    /// ([`context_keys`]), whose value is returned beside its node so the run
    /// can check the built composition carries it whole.
    fn node_values(&self) -> Result<(Vec<NodeValue>, Vec<Routed>), EngineError> {
        let mut values = Vec::new();
        let mut routed = self.expected.clone();
        for written in &self.written {
            let node = self
                .index
                .node_by_flat_id(&written.flat_id)
                .map_err(|source| EngineError::Build {
                    source: Box::new(source),
                })?;
            let refuse = |source: RmError| EngineError::Rm {
                mapping: String::from(node.aql_path().as_str()),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            };
            let value = match written.value {
                Held::Value(ref value) => value.clone(),
                Held::Partial(ref object) => {
                    let rm_type = object
                        .get("_type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(node.rm_type());
                    RmValue::from_canonical(
                        rm_type,
                        node.aql_path().as_str(),
                        &serde_json::Value::Object(object.clone()),
                    )
                    .map_err(refuse)?
                }
            };
            if let Some(keys) = context_keys(node, &value) {
                values.extend(keys);
                routed.push((written.flat_id.clone(), value));
                continue;
            }
            values.push(
                NodeValue::new(node, value.to_canonical().map_err(refuse)?)
                    .with_occurrences(written.positions.clone())
                    .with_datum(rm::RAW),
            );
        }
        values.extend(self.families.iter().cloned());
        values.extend(self.context_keys.iter().cloned());
        Ok((values, routed))
    }

    /// Refuses a composition whose context-set attributes do not carry the
    /// value the run wrote there whole.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::ContextAttribute`] when the built value differs
    /// from the written one, and [`EngineError::Build`] or
    /// [`EngineError::Rm`] when the built value cannot be read back.
    fn carried_whole(
        &self,
        built: &CanonicalComposition,
        routed: &[Routed],
    ) -> Result<(), EngineError> {
        let refuse_build = |source: PathError| EngineError::Build {
            source: Box::new(source),
        };
        for (flat_id, written) in routed {
            let node = self.index.node_by_flat_id(flat_id).map_err(refuse_build)?;
            let path = node.aql_path().as_str();
            let back = self
                .index
                .read(built, node, &[])
                .map_err(refuse_build)?
                .map(|value| RmValue::from_canonical(node.rm_type(), path, &value))
                .transpose()
                .map_err(|source| EngineError::Rm {
                    mapping: String::from(self.program.context().as_str()),
                    node: String::from(path),
                    source: Box::new(source),
                })?;
            if back.as_ref() != Some(written) {
                return Err(EngineError::ContextAttribute {
                    mapping: String::from(self.program.context().as_str()),
                    node: String::from(path),
                });
            }
        }
        Ok(())
    }
}

/// A value written at a composition attribute the FLAT builder sets from the
/// context vocabulary, beside the node it belongs to.
type Routed = (FlatId, RmValue);

// TODO(#241): the three attributes travel as `ctx/` keys until openehr-sdt
// honours `|raw` on the COMPOSITION `language`, `territory` and `composer`
// nodes, which its builder routes through the context (new sibling request S6).
/// Returns the `ctx/` keys a value at a composition attribute the FLAT
/// builder sets from the context vocabulary travels as, `None` for any other
/// node.
///
/// Simplified Formats, master06 §Composer and §Language and Territory:
/// `ctx/composer_name` sets the composer's name and `ctx/language` and
/// `ctx/territory` the two codes. A part the keys do not hold is left out
/// here and refused by [`Run::carried_whole`] once the composition is built.
fn context_keys(node: &ResolvedNode, value: &RmValue) -> Option<Vec<NodeValue>> {
    let key = |field: &str, text: &str| {
        NodeValue::context(field, serde_json::Value::String(String::from(text)))
    };
    let field = match node.aql_path().as_str() {
        "/language" => "language",
        "/territory" => "territory",
        "/composer" => "composer_name",
        _ => return None,
    };
    Some(match *value {
        RmValue::CodePhrase(ref code) if field != "composer_name" => {
            vec![key(field, &code.code_string)]
        }
        RmValue::Party(ref party) if field == "composer_name" => {
            party.name.iter().map(|name| key(field, name)).collect()
        }
        _ => Vec::new(),
    })
}

/// Whether a mapping ran, or its input side kept it from running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ran {
    /// The mapping ran over the occurrences its input admitted.
    Ran,
    /// The unidirectional marker or the input side's condition kept it from
    /// running.
    Gated,
}

/// Returns how many axes of the output side the parent bound.
fn parent_depth(parent: &Binding, direction: Direction) -> usize {
    match direction {
        Direction::FhirToOpenehr => parent.openehr.len(),
        Direction::OpenehrToFhir => parent.fhir.indices().len(),
    }
}

/// Returns the repeating elements a FHIR path steps through, outermost first.
///
/// Each axis is named by the JSON keys the walk took from the resource root,
/// so two elements that share a definition (`Condition.code.coding` and
/// `Condition.verificationStatus.coding` are both `CodeableConcept.coding`)
/// count their instances apart. An `extension(url)` step takes an index of
/// its own, because the url names one entry of a repeating element
/// (<https://hl7.org/fhir/R4/fhirpath.html>, §Additional functions).
fn fhir_axes(target: &FhirTarget) -> Vec<String> {
    let mut walked = String::from(target.resolved().resource());
    let mut axes = Vec::new();
    for step in target.resolved().moves() {
        match *step {
            Move::Member(ref field) => {
                walked.push('.');
                walked.push_str(field.key());
                if field.repeats() {
                    axes.push(walked.clone());
                }
            }
            Move::Choice { ref field, .. } => {
                walked.push('.');
                walked.push_str(field.key());
            }
            Move::Extension { ref field, ref url } => {
                walked.push('.');
                walked.push_str(field.key());
                walked.push('(');
                walked.push_str(url);
                walked.push(')');
                axes.push(walked.clone());
            }
            Move::Ordinal(_) | Move::Index(_) | Move::Predicate(_) | Move::Resolve { .. } => {}
        }
    }
    axes
}

/// Returns the value a target's tail names below its node.
///
/// A Web Template node is as deep as the template constrains, and a target
/// may name an attribute below it, so the tail is read from the canonical
/// JSON of the node with `PATHABLE.item_at_path` of `openehr-rm`.
fn attribute_at<'value>(
    value: &'value serde_json::Value,
    target: &OpenehrTarget,
) -> Option<&'value serde_json::Value> {
    item_at_path(value, target.tail())
}

/// Drops each group-independent warning declared from `start` on that an
/// earlier warning from `start` on already declares.
fn settle(warnings: &mut Vec<Warning>, start: usize) {
    let start = start.min(warnings.len());
    let declared = warnings.split_off(start);
    for warning in declared {
        let alike = matches!(
            warning,
            Warning::Skipped {
                reason: SkipReason::Unidirectional,
                ..
            }
        );
        let seen = warnings
            .get(start..)
            .is_some_and(|kept| kept.contains(&warning));
        if alike && seen {
            continue;
        }
        warnings.push(warning);
    }
}

/// Returns the openEHR positions an openEHR input occurrence carries.
///
/// An openEHR occurrence holds 1-based positions (openEHR BASE Release 1.2.0
/// §Paths and Locators), so an index past `u32` is
/// [`PositionError::Overflow`] and a 0 is [`PositionError::Zero`], each a
/// refusal naming the mapping and never a dropped position.
fn rm_positions(name: &str, indices: &[usize]) -> Result<Vec<RmPosition>, EngineError> {
    let refuse = |source: PositionError| EngineError::Position {
        mapping: String::from(name),
        source,
    };
    let mut positions = Vec::with_capacity(indices.len());
    for &index in indices {
        let index = u32::try_from(index).map_err(|_refused| refuse(PositionError::Overflow))?;
        positions.push(RmPosition::new(index).map_err(refuse)?);
    }
    Ok(positions)
}

/// Returns an openEHR input occurrence of the positions it names.
fn openehr_occurrence(name: &str, positions: &[RmPosition]) -> Result<Occurrence, EngineError> {
    let mut indices = Vec::with_capacity(positions.len());
    for position in positions {
        indices.push(usize::try_from(position.get()).map_err(|_refused| {
            EngineError::Position {
                mapping: String::from(name),
                source: PositionError::Overflow,
            }
        })?);
    }
    Ok(Occurrence::new(indices))
}

/// Returns the text one FHIR value is compared by, for a `unique` tuple.
fn lexical(value: &Value) -> String {
    match *value {
        Value::Null => String::new(),
        Value::Bool(flag) => String::from(if flag { "true" } else { "false" }),
        Value::Number(ref number) => String::from(number.as_str()),
        Value::String(ref text) => text.clone(),
        Value::Array(ref items) => items.iter().map(lexical).collect::<Vec<String>>().join(","),
        Value::Object(ref object) => object
            .iter()
            .map(|(key, member)| format!("{key}={}", lexical(member)))
            .collect::<Vec<String>>()
            .join(";"),
    }
}

/// Returns the attribute names a target's tail walks, outermost first.
fn tail_segments(target: &OpenehrTarget) -> Vec<&str> {
    target
        .tail()
        .segments
        .iter()
        .map(|segment| segment.attribute.as_str())
        .collect()
}

/// Returns the refusal of a tail no FLAT part of the node's class carries.
fn unsupported_tail(mapping: &Mapping, target: &OpenehrTarget) -> EngineError {
    EngineError::UnsupportedTail {
        mapping: String::from(mapping.name()),
        node: String::from(target.node().aql_path().as_str()),
        tail: target.tail().to_string(),
    }
}

/// Merges one value into an object at an attribute path.
///
/// An intermediate attribute that holds no object is replaced by one, since
/// a path below it names its members.
fn merge_value(
    object: &mut serde_json::Map<String, serde_json::Value>,
    segments: &[&str],
    value: serde_json::Value,
) {
    let Some((first, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        object.insert(String::from(*first), value);
        return;
    }
    let entry = object
        .entry(String::from(*first))
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !entry.is_object() {
        *entry = serde_json::Value::Object(serde_json::Map::new());
    }
    if let serde_json::Value::Object(ref mut nested) = *entry {
        merge_value(nested, rest, value);
    }
}

/// Returns the canonical JSON of one scalar tail attribute, from the FHIR
/// element's JSON.
///
/// The attribute's type comes from the RM attribute model, so a text that
/// does not read as it is a refusal naming the class.
fn scalar_json(
    carried: Carried,
    class: &str,
    node: &ResolvedNode,
    written: &Value,
) -> Result<serde_json::Value, RmError> {
    let text = match *written {
        Value::String(ref text) => text.clone(),
        Value::Number(ref number) => String::from(number.as_str()),
        Value::Bool(flag) => String::from(if flag { "true" } else { "false" }),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    };
    let refuse = |expected: &'static str| RmError::Scalar {
        node: String::from(node.aql_path().as_str()),
        rm_type: String::from(class),
        text: text.clone(),
        expected,
    };
    match carried {
        Carried::Text if written.as_str().is_some() => Ok(serde_json::Value::String(text)),
        Carried::Text | Carried::Value | Carried::Family => Err(refuse("text")),
        Carried::Real => serde_json::from_str::<serde_json::Number>(&text)
            .map(serde_json::Value::Number)
            .map_err(|_refused| refuse("a real number")),
        Carried::Integer => text
            .parse::<i64>()
            .map(|number| serde_json::Value::Number(number.into()))
            .map_err(|_refused| refuse("an integer")),
        Carried::Boolean => match text.as_str() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err(refuse("a boolean")),
        },
    }
}

/// Returns the text a scalar carries, `None` for a structure.
fn scalar(value: &serde_json::Value) -> Option<String> {
    match *value {
        serde_json::Value::String(ref text) => Some(text.clone()),
        serde_json::Value::Bool(flag) => Some(String::from(if flag { "true" } else { "false" })),
        serde_json::Value::Number(ref number) => Some(number.to_string()),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            None
        }
    }
}

/// Renders an index prefix, for the counter that appends under it.
fn render(indices: &[usize]) -> String {
    indices
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(".")
}

/// Merges one manual path into the object the entry builds.
fn merge(object: &mut serde_json::Map<String, serde_json::Value>, segments: &[&str], value: &str) {
    let Some((first, rest)) = segments.split_first() else {
        object.insert(
            String::from("value"),
            serde_json::Value::String(String::from(value)),
        );
        return;
    };
    if rest.is_empty() {
        object.insert(
            String::from(*first),
            serde_json::Value::String(String::from(value)),
        );
        return;
    }
    let entry = object
        .entry(String::from(*first))
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let serde_json::Value::Object(ref mut nested) = *entry {
        merge(nested, rest, value);
    }
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use super::EngineError;
    use super::MappingFunctions;
    use super::NoMappingFunctions;
    use super::fhir_axes;
    use super::merge;
    use super::render;
    use super::rm_positions;
    use super::settle;
    use crate::engine::outcome::SkipReason;
    use crate::engine::outcome::Warning;
    use crate::resolve::program::FhirTarget;
    use crate::tree::element::resolve;
    use crate::tree::path::FhirPath;
    use openehr_mapping_core::composition::PositionError;

    /// Resolves `expression` against the R4 `Condition`.
    fn target(expression: &str) -> FhirTarget {
        let path = FhirPath::from_str(expression).expect("the expression parses");
        let resolved = resolve(&fhir_types::r4::schema::SCHEMAS, "Condition", &path)
            .expect("the expression resolves");
        FhirTarget::new(path, resolved)
    }

    #[test]
    fn two_elements_that_share_a_definition_count_their_instances_apart() {
        // Condition.code and Condition.verificationStatus are both
        // CodeableConcept, so both codings are `CodeableConcept.coding`.
        let code = fhir_axes(&target("$resource.code.coding.code"));
        let status = fhir_axes(&target("$resource.verificationStatus.coding.code"));
        assert_eq!(code, vec![String::from("Condition.code.coding")]);
        assert_eq!(
            status,
            vec![String::from("Condition.verificationStatus.coding")]
        );
    }

    #[test]
    fn the_registry_this_milestone_ships_holds_nothing() {
        let error = NoMappingFunctions
            .to_openehr("diagnosis", None)
            .expect_err("the registry ships empty");
        assert!(
            error.to_string().contains("diagnosis"),
            "the refusal names the function: {error}"
        );
    }

    #[test]
    fn manual_paths_that_share_a_prefix_merge_into_one_element() {
        // manual.adoc: "`defining_code/terminology_id/value` and
        // `defining_code/code_string` are merged together in the same data
        // element and do not overwrite".
        let mut object = serde_json::Map::new();
        merge(
            &mut object,
            &["defining_code", "terminology_id", "value"],
            "openehr",
        );
        merge(&mut object, &["defining_code", "code_string"], "524");
        merge(&mut object, &["value"], "Initial");
        assert_eq!(
            serde_json::Value::Object(object),
            serde_json::json!({
                "defining_code": {"terminology_id": {"value": "openehr"}, "code_string": "524"},
                "value": "Initial"
            })
        );
    }

    #[test]
    fn an_index_past_u32_is_an_overflow_and_never_a_zero() {
        let past = usize::try_from(u64::from(u32::MAX) + 1).expect("a 64-bit usize");
        let error = rm_positions("site", &[1, past]).expect_err("the index has no position");
        assert!(
            matches!(
                error,
                EngineError::Position {
                    source: PositionError::Overflow,
                    ..
                }
            ),
            "an overflow names itself: {error}"
        );
    }

    #[test]
    fn a_zero_position_is_refused_and_never_dropped() {
        let error = rm_positions("site", &[1, 0]).expect_err("0 is no openEHR position");
        assert!(matches!(
            error,
            EngineError::Position {
                source: PositionError::Zero,
                ..
            }
        ));
        assert_eq!(
            rm_positions("site", &[2, 1])
                .expect("both are positions")
                .iter()
                .map(|position| position.get())
                .collect::<Vec<u32>>(),
            [2, 1]
        );
    }

    #[test]
    fn a_split_declares_a_group_independent_skip_once_and_keeps_the_rest() {
        let skip = Warning::Skipped {
            mapping: String::from("subject"),
            reason: SkipReason::Unidirectional,
        };
        let dropped = Warning::LastOfMany {
            path: String::from("/content"),
            dropped: 1,
        };
        let before = Warning::Defaulted {
            field: String::from("/composer"),
        };
        let mut warnings = vec![
            before.clone(),
            skip.clone(),
            dropped.clone(),
            skip.clone(),
            dropped.clone(),
        ];
        settle(&mut warnings, 1);
        assert_eq!(warnings, [before, skip, dropped.clone(), dropped]);
    }

    #[test]
    fn an_index_prefix_renders_as_the_counter_key() {
        assert_eq!(render(&[]), "");
        assert_eq!(render(&[0, 2]), "0.2");
    }
}
