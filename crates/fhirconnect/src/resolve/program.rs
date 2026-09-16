// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The immutable output of compiling one context mapping.
//!
//! A [`Program`] is built once by [`crate::resolve::compile`] and never
//! changes afterwards: every field is private, no method takes `&mut self`,
//! and the type is handed around behind an `Arc`. Everything the interpreter
//! needs is already resolved here, so running a mapping parses no path and
//! reads no mapping file.
//!
//! The rendering [`Program`] writes through [`fmt::Display`] is the shape the
//! snapshot tests pin. It is a plain indented tree rather than a
//! serialization, because nothing outside a test reads a program back.

use core::fmt;
use core::str::FromStr;

use openehr_mapping_core::header::ArchetypeId;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::header::MappingVersion;
use openehr_mapping_core::index::FlatId;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::template::Generation;
use openehr_rm::v1_2::paths::RmPath;

use crate::model::ast::ConditionOperator;
use crate::model::ast::DataType;
use crate::model::ast::Direction;
use crate::tree::element::Location;
use crate::tree::element::Move;
use crate::tree::element::Resolved;
use crate::tree::path::FhirPath;
use crate::tree::path::Writability;

/// The canonical URL of a FHIR profile, as a context mapping writes it.
///
/// A canonical URL may carry a version after a `|`
/// (<https://hl7.org/fhir/R4/references.html#canonical>); this type holds the
/// URL alone, and the version travels beside it in [`ProfileBinding`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileUrl(String);

impl ProfileUrl {
    /// Wraps a profile canonical URL.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// Returns the URL.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The identifier of an operational template, as a context mapping writes it.
///
/// This is the openEHR side of the selection and never carries a FHIR
/// identifier, so a swapped argument is a compile error rather than a wrong
/// program.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TemplateId(String);

impl TemplateId {
    /// Wraps a template identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TemplateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The FHIR resource type a program reads and writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceType(String);

impl ResourceType {
    /// Wraps a resource type name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the resource type name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A version selector, which a mapping file may leave out.
///
/// The context schema makes `profile.version` and `template.sem_ver` optional,
/// so a program records what the files pinned and says so where they pinned
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pin {
    /// The file names a version, and the compiler checked what it could.
    Pinned(String),
    /// The file names no version.
    Unpinned,
}

impl Pin {
    /// Wraps an optional version.
    #[must_use]
    pub fn new(version: Option<&str>) -> Self {
        version.map_or(Self::Unpinned, |value| Self::Pinned(value.to_owned()))
    }

    /// Returns the pinned version, `None` when the file pinned none.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        match *self {
            Self::Pinned(ref version) => Some(version),
            Self::Unpinned => None,
        }
    }

    /// Whether the file pinned no version.
    #[must_use]
    pub const fn is_unpinned(&self) -> bool {
        matches!(*self, Self::Unpinned)
    }
}

impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Pinned(ref version) => f.write_str(version),
            Self::Unpinned => f.write_str("unpinned"),
        }
    }
}

/// The profile a program maps, as the context mapping pins it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileBinding {
    url: ProfileUrl,
    version: Pin,
}

impl ProfileBinding {
    /// Binds a profile URL to the version the context pins.
    #[must_use]
    pub const fn new(url: ProfileUrl, version: Pin) -> Self {
        Self { url, version }
    }

    /// Returns the profile URL.
    #[must_use]
    pub const fn url(&self) -> &ProfileUrl {
        &self.url
    }

    /// Returns the pinned profile version.
    #[must_use]
    pub const fn version(&self) -> &Pin {
        &self.version
    }
}

impl fmt::Display for ProfileBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} version {}", self.url, self.version)
    }
}

/// The operational template a program maps, as the context mapping pins it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateBinding {
    id: TemplateId,
    sem_ver: Pin,
    generation: Generation,
}

impl TemplateBinding {
    /// Binds a template identifier to the version the context pins.
    #[must_use]
    pub const fn new(id: TemplateId, sem_ver: Pin, generation: Generation) -> Self {
        Self {
            id,
            sem_ver,
            generation,
        }
    }

    /// Returns the template identifier.
    #[must_use]
    pub const fn id(&self) -> &TemplateId {
        &self.id
    }

    /// Returns the pinned template version.
    #[must_use]
    pub const fn sem_ver(&self) -> &Pin {
        &self.sem_ver
    }

    /// Returns the generation the template was served in.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }
}

impl fmt::Display for TemplateBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} sem_ver {} generation {}",
            self.id, self.sem_ver, self.generation
        )
    }
}

/// One model mapping a program compiled, with the versions it was compiled
/// against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelBinding {
    name: MappingName,
    version: MappingVersion,
    archetype: Option<ArchetypeId>,
    revision: Pin,
    extensions: Vec<MappingName>,
}

impl ModelBinding {
    /// Records one compiled model mapping.
    #[must_use]
    pub const fn new(
        name: MappingName,
        version: MappingVersion,
        archetype: Option<ArchetypeId>,
        revision: Pin,
        extensions: Vec<MappingName>,
    ) -> Self {
        Self {
            name,
            version,
            archetype,
            revision,
            extensions,
        }
    }

    /// Returns the `metadata.name` of the model mapping.
    #[must_use]
    pub const fn name(&self) -> &MappingName {
        &self.name
    }

    /// Returns the `metadata.version` the compiled file carried.
    #[must_use]
    pub const fn version(&self) -> &MappingVersion {
        &self.version
    }

    /// Returns the archetype the model mapping declares, which an extension
    /// file declares none of.
    #[must_use]
    pub const fn archetype(&self) -> Option<&ArchetypeId> {
        self.archetype.as_ref()
    }

    /// Returns the archetype revision the model mapping pins.
    #[must_use]
    pub const fn revision(&self) -> &Pin {
        &self.revision
    }

    /// Returns the extensions applied to it, in the order they applied.
    #[must_use]
    pub fn extensions(&self) -> &[MappingName] {
        &self.extensions
    }
}

/// A `with.fhir` expression, bound to its anchor and resolved against the
/// element table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FhirTarget {
    expression: FhirPath,
    resolved: Resolved,
}

impl FhirTarget {
    /// Pairs an anchored expression with its element-table resolution.
    #[must_use]
    pub const fn new(expression: FhirPath, resolved: Resolved) -> Self {
        Self {
            expression,
            resolved,
        }
    }

    /// Returns the `$resource`-rooted expression.
    #[must_use]
    pub const fn expression(&self) -> &FhirPath {
        &self.expression
    }

    /// Returns the element-table resolution.
    #[must_use]
    pub const fn resolved(&self) -> &Resolved {
        &self.resolved
    }

    /// Returns whether the expression can be written through.
    #[must_use]
    pub const fn writability(&self) -> &Writability {
        self.expression.writability()
    }

    /// Returns whether the expression names many values.
    ///
    /// The element the walk ends on decides it, unless a later step picks one
    /// of the values a document already holds: an index, an ordinal, a
    /// predicate, or the `extension(url)` shortcut, whose url "is the
    /// identity" of one extension
    /// (<https://hl7.org/fhir/R4/extensibility.html>). This is the FHIR-side
    /// counterpart of [`OpenehrTarget::occurrences`].
    #[must_use]
    pub fn repeats(&self) -> bool {
        let mut repeats = false;
        for step in self.resolved.moves() {
            match *step {
                Move::Member(ref field) | Move::Choice { ref field, .. } => {
                    repeats = field.repeats();
                }
                Move::Extension { .. } | Move::Ordinal(_) | Move::Index(_) | Move::Predicate(_) => {
                    repeats = false;
                }
                Move::Resolve { .. } => {}
            }
        }
        repeats
    }
}

impl fmt::Display for FhirTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let writability = match *self.writability() {
            Writability::Writable => "writable",
            Writability::ReadOnly { .. } => "read-only",
        };
        write!(
            f,
            "{} -> {} {} {}",
            self.expression,
            self.resolved.leaf(),
            location_name(self.resolved.location()),
            writability
        )
    }
}

/// Names where a resolved FHIR path ends, for a rendering that stays stable.
fn location_name(location: &Location) -> &'static str {
    match *location {
        Location::Complex(_) => "complex",
        Location::Primitive(_) => "primitive",
        Location::Attribute => "attribute",
        Location::PrimitiveElement => "primitive-element",
        Location::Choice(_) => "choice",
        Location::Resource => "resource",
        Location::Deferred => "deferred",
    }
}

/// A `with.openehr` path, bound to its anchor and resolved against the Web
/// Template.
///
/// The Web Template indexes the nodes a template constrains, and an openEHR
/// path may reach below the deepest of them into the reference model, so a
/// target carries the node plus the attribute tail under it. No specification
/// governs the meeting of a mapping path and a Web Template: this split is
/// FerroBRIDGE's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenehrTarget {
    path: RmPath,
    node: ResolvedNode,
    tail: RmPath,
    occurrences: Vec<FlatId>,
}

impl OpenehrTarget {
    /// Pairs a resolved path with the template node it names.
    #[must_use]
    pub const fn new(
        path: RmPath,
        node: ResolvedNode,
        tail: RmPath,
        occurrences: Vec<FlatId>,
    ) -> Self {
        Self {
            path,
            node,
            tail,
            occurrences,
        }
    }

    /// Returns the absolute openEHR path the mapping names.
    #[must_use]
    pub const fn path(&self) -> &RmPath {
        &self.path
    }

    /// Returns the template node the path resolves to.
    #[must_use]
    pub const fn node(&self) -> &ResolvedNode {
        &self.node
    }

    /// Returns the reference-model attributes below the node, empty when the
    /// path names the node itself.
    #[must_use]
    pub const fn tail(&self) -> &RmPath {
        &self.tail
    }

    /// Returns the repeating nodes on the way to this one, outermost first.
    ///
    /// One occurrence index belongs to each of them, which is what makes an
    /// occurrence a structured index rather than a pattern over a rendered
    /// path.
    #[must_use]
    pub fn occurrences(&self) -> &[FlatId] {
        &self.occurrences
    }
}

impl fmt::Display for OpenehrTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} -> {} {}",
            self.path,
            self.node.aql_path().as_str(),
            self.node.rm_type()
        )?;
        if !self.tail.segments.is_empty() {
            write!(f, " tail {}", self.tail)?;
        }
        if !self.occurrences.is_empty() {
            let axes: Vec<&str> = self.occurrences.iter().map(FlatId::as_str).collect();
            write!(f, " occurrences [{}]", axes.join(", "))?;
        }
        Ok(())
    }
}

/// One side of a mapping, on the side the path belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A FHIR expression.
    Fhir(Box<FhirTarget>),
    /// An openEHR path.
    Openehr(Box<OpenehrTarget>),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Fhir(ref target) => write!(f, "fhir {target}"),
            Self::Openehr(ref target) => write!(f, "openehr {target}"),
        }
    }
}

/// How a condition's `targetRoot` stands to the path it guards.
///
/// The `targetRoot` "defines what element is returned once filtered ... the
/// returned element is matched against the path in the `with` method", and a
/// condition may also "be unattached to the path contained in the `with:`
/// method and point to a different path", which is "handled as simple
/// true/false"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §targetRoot). The compiler decides which of the four it is, over the
/// anchored paths, so the engine never compares path text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Attachment {
    /// The `targetRoot` is the guarded path, so the condition filters the
    /// values that path maps.
    Element,
    /// The `targetRoot` is above the guarded path.
    Ancestor,
    /// The `targetRoot` is below the guarded path, which is the shape a
    /// preprocessor gate writes and a mapping-level condition is refused for.
    Descendant,
    /// Neither path contains the other, so the condition is a plain true or
    /// false test.
    Unrelated,
}

impl fmt::Display for Attachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Element => f.write_str("element"),
            Self::Ancestor => f.write_str("ancestor"),
            Self::Descendant => f.write_str("descendant"),
            Self::Unrelated => f.write_str("unrelated"),
        }
    }
}

/// The parts of a compiled condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionParts {
    /// The direction that evaluates the condition.
    pub direction: Direction,
    /// The resolved `targetRoot`.
    pub target: Target,
    /// The resolved target attributes.
    pub attributes: Vec<Target>,
    /// The operator.
    pub operator: ConditionOperator,
    /// The values the operator tests against.
    pub criteria: Vec<String>,
    /// Whether the condition identifies the element it guards.
    pub identifying: bool,
    /// How the `targetRoot` stands to the path the condition guards.
    pub attachment: Attachment,
}

/// A compiled `fhirCondition` or `openehrCondition`.
///
/// A condition is evaluated on the input side only, so [`Condition::direction`]
/// says which direction runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    direction: Direction,
    target: Target,
    attributes: Vec<Target>,
    operator: ConditionOperator,
    criteria: Vec<String>,
    identifying: bool,
    attachment: Attachment,
}

impl Condition {
    /// Assembles a compiled condition.
    #[must_use]
    pub fn new(parts: ConditionParts) -> Self {
        Self {
            direction: parts.direction,
            target: parts.target,
            attributes: parts.attributes,
            operator: parts.operator,
            criteria: parts.criteria,
            identifying: parts.identifying,
            attachment: parts.attachment,
        }
    }

    /// Returns how the `targetRoot` stands to the path this condition guards.
    #[must_use]
    pub const fn attachment(&self) -> Attachment {
        self.attachment
    }

    /// Returns the direction that evaluates this condition.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Returns the resolved `targetRoot`.
    #[must_use]
    pub const fn target(&self) -> &Target {
        &self.target
    }

    /// Returns the resolved target attributes, which are tested with OR.
    #[must_use]
    pub fn attributes(&self) -> &[Target] {
        &self.attributes
    }

    /// Returns the operator.
    #[must_use]
    pub const fn operator(&self) -> ConditionOperator {
        self.operator
    }

    /// Returns the criteria the operator tests against.
    #[must_use]
    pub fn criteria(&self) -> &[String] {
        &self.criteria
    }

    /// Whether the condition identifies the element it guards.
    #[must_use]
    pub const fn identifying(&self) -> bool {
        self.identifying
    }
}

/// What a `manual` entry writes at a resolved path.
///
/// A value is a literal unless it names a `$context` member, which "holds
/// values passed in on the REST call ... a context value is referenced from a
/// `manual` `value`"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`,
/// §`$context`), so the two are told apart when the mapping is compiled rather
/// than on every request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ManualValue {
    /// The text the mapping wrote.
    Literal(String),
    /// The name of the `$context` member the caller supplies.
    Context(String),
}

impl fmt::Display for ManualValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Literal(ref text) => f.write_str(text),
            Self::Context(ref name) => write!(f, "$context.{name}"),
        }
    }
}

/// One value a `manual` entry writes at a resolved path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPath {
    target: Target,
    value: ManualValue,
}

impl ManualPath {
    /// Pairs a resolved path with the value written there.
    #[must_use]
    pub const fn new(target: Target, value: ManualValue) -> Self {
        Self { target, value }
    }

    /// Returns the resolved path.
    #[must_use]
    pub const fn target(&self) -> &Target {
        &self.target
    }

    /// Returns the value.
    #[must_use]
    pub const fn value(&self) -> &ManualValue {
        &self.value
    }
}

/// A compiled `manual` entry.
///
/// The paths inside one entry merge into one element
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/manual.adoc`),
/// so they travel together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manual {
    name: String,
    fhir: Vec<ManualPath>,
    openehr: Vec<ManualPath>,
    fhir_condition: Option<Condition>,
    openehr_condition: Option<Condition>,
    value: Option<String>,
    direction: Option<Direction>,
}

impl Manual {
    /// Assembles a compiled manual entry.
    #[must_use]
    pub const fn new(
        name: String,
        fhir: Vec<ManualPath>,
        openehr: Vec<ManualPath>,
        fhir_condition: Option<Condition>,
        openehr_condition: Option<Condition>,
        value: Option<String>,
        direction: Option<Direction>,
    ) -> Self {
        Self {
            name,
            fhir,
            openehr,
            fhir_condition,
            openehr_condition,
            value,
            direction,
        }
    }

    /// Returns the entry name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the FHIR literals of the entry.
    #[must_use]
    pub fn fhir(&self) -> &[ManualPath] {
        &self.fhir
    }

    /// Returns the openEHR literals of the entry.
    #[must_use]
    pub fn openehr(&self) -> &[ManualPath] {
        &self.openehr
    }

    /// Returns the condition the FHIR to openEHR direction evaluates.
    #[must_use]
    pub const fn fhir_condition(&self) -> Option<&Condition> {
        self.fhir_condition.as_ref()
    }

    /// Returns the condition the openEHR to FHIR direction evaluates.
    #[must_use]
    pub const fn openehr_condition(&self) -> Option<&Condition> {
        self.openehr_condition.as_ref()
    }

    /// Returns the value the entry writes, when it writes one value rather
    /// than a set of paths.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Returns the one direction the entry runs in, when it is pinned to one.
    #[must_use]
    pub const fn direction(&self) -> Option<Direction> {
        self.direction
    }
}

/// What a compiled mapping does beyond mapping its two paths against each
/// other.
///
/// The concept-type chapter names eight kinds of mapping and each is decided
/// by the key the mapping writes
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// The two paths are mapped against each other.
    Value,
    /// The mapping hands over to another model mapping, expanded here.
    Slot {
        /// The model mapping the slot names.
        model: MappingName,
        /// The preprocessor of that file and of every extension applied to it,
        /// in application order, which gates the slotted mappings.
        preprocessors: Vec<Preprocessor>,
        /// Its compiled mappings, under this mapping's anchors.
        mappings: Vec<Mapping>,
    },
    /// The mapping initializes another resource, whose mappings follow.
    Reference {
        /// The resource type the reference names.
        resource: ResourceType,
        /// The mappings that populate it.
        mappings: Vec<Mapping>,
    },
    /// The mapping writes an openEHR `LINK`.
    Link {
        /// The meaning the link carries.
        meaning: Option<String>,
        /// The type the link carries.
        link_type: Option<String>,
    },
    /// The mapping calls a registered function.
    Programmed {
        /// The name of the function.
        code: String,
    },
    /// The mapping fills a participation function.
    Participation {
        /// The function the participation carries.
        function: String,
    },
}

/// One compiled mapping method, with everything the interpreter needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    name: String,
    model: MappingName,
    fhir: Option<FhirTarget>,
    openehr: Option<OpenehrTarget>,
    data_type: Option<DataType>,
    value: Option<String>,
    direction: Option<Direction>,
    fhir_condition: Option<Condition>,
    openehr_condition: Option<Condition>,
    manual: Vec<Manual>,
    conceptmap: Option<String>,
    method: Method,
    followed_by: Vec<Mapping>,
}

/// Everything a compiled mapping carries, as the compiler assembles it.
///
/// The compiler fills one of these and hands it to [`Mapping::new`], which
/// keeps a function from taking a dozen positional arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingParts {
    /// The dotted name of the mapping inside its model mapping.
    pub name: String,
    /// The model mapping the method was read from.
    pub model: MappingName,
    /// The resolved FHIR side.
    pub fhir: Option<FhirTarget>,
    /// The resolved openEHR side.
    pub openehr: Option<OpenehrTarget>,
    /// The data type the mapping pins.
    pub data_type: Option<DataType>,
    /// The literal value the mapping writes.
    pub value: Option<String>,
    /// The one direction the mapping runs in, when it is pinned to one.
    pub direction: Option<Direction>,
    /// The condition the FHIR to openEHR direction evaluates.
    pub fhir_condition: Option<Condition>,
    /// The condition the openEHR to FHIR direction evaluates.
    pub openehr_condition: Option<Condition>,
    /// The manual entries of the mapping.
    pub manual: Vec<Manual>,
    /// The `ConceptMap.url` the codes of this method are translated through,
    /// the method's own or the one its file's header attaches.
    pub conceptmap: Option<String>,
    /// What the mapping does beyond mapping its two paths.
    pub method: Method,
    /// The mappings that run after this one, under its anchors.
    pub followed_by: Vec<Mapping>,
}

impl Mapping {
    /// Assembles a compiled mapping.
    #[must_use]
    pub fn new(parts: MappingParts) -> Self {
        Self {
            name: parts.name,
            model: parts.model,
            fhir: parts.fhir,
            openehr: parts.openehr,
            data_type: parts.data_type,
            value: parts.value,
            direction: parts.direction,
            fhir_condition: parts.fhir_condition,
            openehr_condition: parts.openehr_condition,
            manual: parts.manual,
            conceptmap: parts.conceptmap,
            method: parts.method,
            followed_by: parts.followed_by,
        }
    }

    /// Returns the dotted name of the mapping inside its model mapping.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the model mapping the method was read from.
    #[must_use]
    pub const fn model(&self) -> &MappingName {
        &self.model
    }

    /// Returns the resolved FHIR side.
    #[must_use]
    pub const fn fhir(&self) -> Option<&FhirTarget> {
        self.fhir.as_ref()
    }

    /// Returns the resolved openEHR side.
    #[must_use]
    pub const fn openehr(&self) -> Option<&OpenehrTarget> {
        self.openehr.as_ref()
    }

    /// Returns the data type the mapping pins.
    #[must_use]
    pub const fn data_type(&self) -> Option<DataType> {
        self.data_type
    }

    /// Returns the literal value the mapping writes.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Returns the one direction the mapping runs in, when it is pinned.
    #[must_use]
    pub const fn direction(&self) -> Option<Direction> {
        self.direction
    }

    /// Returns the condition the FHIR to openEHR direction evaluates.
    #[must_use]
    pub const fn fhir_condition(&self) -> Option<&Condition> {
        self.fhir_condition.as_ref()
    }

    /// Returns the condition the openEHR to FHIR direction evaluates.
    #[must_use]
    pub const fn openehr_condition(&self) -> Option<&Condition> {
        self.openehr_condition.as_ref()
    }

    /// Returns the manual entries of the mapping.
    #[must_use]
    pub fn manual(&self) -> &[Manual] {
        &self.manual
    }

    /// Returns the concept map the codes of this mapping translate through.
    ///
    /// The method's own `conceptmap` wins and a `spec.conceptmap` of the file
    /// that contributed the method is the fallback: "the concept map can also
    /// be directly attached inside the header, this way all codes contained in
    /// the conceptmap will be transformed using the conceptmap"
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/manual.adoc`,
    /// §`ConceptMaps`).
    #[must_use]
    pub fn conceptmap(&self) -> Option<&str> {
        self.conceptmap.as_deref()
    }

    /// Returns what the mapping does beyond mapping its two paths.
    #[must_use]
    pub const fn method(&self) -> &Method {
        &self.method
    }

    /// Returns the mappings that run after this one.
    #[must_use]
    pub fn followed_by(&self) -> &[Mapping] {
        &self.followed_by
    }

    /// Writes this mapping and everything under it at `depth`.
    fn render(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        let pad = "  ".repeat(depth);
        writeln!(f, "{pad}mapping {} from {}", self.name, self.model)?;
        let inner = "  ".repeat(depth.saturating_add(1));
        if let Some(ref fhir) = self.fhir {
            writeln!(f, "{inner}fhir {fhir}")?;
        }
        if let Some(ref openehr) = self.openehr {
            writeln!(f, "{inner}openehr {openehr}")?;
        }
        if let Some(data_type) = self.data_type {
            writeln!(f, "{inner}type {data_type}")?;
        }
        if let Some(ref value) = self.value {
            writeln!(f, "{inner}value {value}")?;
        }
        if let Some(direction) = self.direction {
            writeln!(f, "{inner}unidirectional {direction}")?;
        }
        if let Some(ref conceptmap) = self.conceptmap {
            writeln!(f, "{inner}conceptmap {conceptmap}")?;
        }
        for (label, condition) in [
            ("fhirCondition", self.fhir_condition.as_ref()),
            ("openehrCondition", self.openehr_condition.as_ref()),
        ] {
            if let Some(condition) = condition {
                render_condition(f, &inner, label, condition)?;
            }
        }
        for manual in &self.manual {
            render_manual(f, &inner, manual)?;
        }
        match self.method {
            Method::Value => {}
            Method::Slot {
                ref model,
                ref preprocessors,
                ref mappings,
            } => {
                writeln!(f, "{inner}slotArchetype {model}")?;
                let under = "  ".repeat(depth.saturating_add(2));
                for preprocessor in preprocessors {
                    render_preprocessor(f, &under, preprocessor)?;
                }
                for mapping in mappings {
                    mapping.render(f, depth.saturating_add(2))?;
                }
            }
            Method::Reference {
                ref resource,
                ref mappings,
            } => {
                writeln!(f, "{inner}reference {resource}")?;
                for mapping in mappings {
                    mapping.render(f, depth.saturating_add(2))?;
                }
            }
            Method::Link {
                ref meaning,
                ref link_type,
            } => {
                writeln!(
                    f,
                    "{inner}link meaning {} type {}",
                    meaning.as_deref().unwrap_or("-"),
                    link_type.as_deref().unwrap_or("-")
                )?;
            }
            Method::Programmed { ref code } => writeln!(f, "{inner}mappingCode {code}")?,
            Method::Participation { ref function } => {
                writeln!(f, "{inner}participationsFunction {function}")?;
            }
        }
        for child in &self.followed_by {
            child.render(f, depth.saturating_add(1))?;
        }
        Ok(())
    }
}

/// Writes one file's compiled preprocessor.
fn render_preprocessor(
    f: &mut fmt::Formatter<'_>,
    pad: &str,
    preprocessor: &Preprocessor,
) -> fmt::Result {
    if preprocessor.is_empty() {
        return Ok(());
    }
    writeln!(f, "{pad}preprocessor {}", preprocessor.model())?;
    let inner = format!("{pad}  ");
    for (label, condition) in [
        ("fhirCondition", preprocessor.fhir_condition()),
        ("openehrCondition", preprocessor.openehr_condition()),
    ] {
        if let Some(condition) = condition {
            render_condition(f, &inner, label, condition)?;
        }
    }
    if let Some(hierarchy) = preprocessor.hierarchy() {
        writeln!(f, "{inner}hierarchy")?;
        if let Some(fhir) = hierarchy.fhir() {
            writeln!(f, "{inner}  fhir {fhir}")?;
        }
        if let Some(openehr) = hierarchy.openehr() {
            writeln!(f, "{inner}  openehr {openehr}")?;
        }
        for (label, split) in [
            ("split fhir", hierarchy.split_fhir()),
            ("split openehr", hierarchy.split_openehr()),
        ] {
            let Some(split) = split else {
                continue;
            };
            writeln!(
                f,
                "{inner}  {label} create {}",
                split.create().map_or("-", Create::as_str)
            )?;
            if let Some(path) = split.path() {
                writeln!(f, "{inner}    path {path}")?;
            }
            for unique in split.unique() {
                writeln!(f, "{inner}    unique {unique}")?;
            }
        }
    }
    Ok(())
}

/// Writes one compiled condition.
fn render_condition(
    f: &mut fmt::Formatter<'_>,
    pad: &str,
    label: &str,
    condition: &Condition,
) -> fmt::Result {
    writeln!(
        f,
        "{pad}{label} {} {} on {} ({})",
        condition.operator(),
        render_criteria(condition.criteria()),
        condition.target(),
        condition.attachment()
    )?;
    for attribute in condition.attributes() {
        writeln!(f, "{pad}  attribute {attribute}")?;
    }
    Ok(())
}

/// Renders a criteria list as one stable string.
fn render_criteria(criteria: &[String]) -> String {
    if criteria.is_empty() {
        return String::from("[]");
    }
    format!("[{}]", criteria.join(", "))
}

/// Writes one compiled manual entry.
fn render_manual(f: &mut fmt::Formatter<'_>, pad: &str, manual: &Manual) -> fmt::Result {
    writeln!(f, "{pad}manual {}", manual.name())?;
    let inner = format!("{pad}  ");
    if let Some(direction) = manual.direction() {
        writeln!(f, "{inner}unidirectional {direction}")?;
    }
    if let Some(value) = manual.value() {
        writeln!(f, "{inner}value {value}")?;
    }
    for path in manual.fhir().iter().chain(manual.openehr()) {
        writeln!(f, "{inner}{} = {}", path.target(), path.value())?;
    }
    for (label, condition) in [
        ("fhirCondition", manual.fhir_condition()),
        ("openehrCondition", manual.openehr_condition()),
    ] {
        if let Some(condition) = condition {
            render_condition(f, &inner, label, condition)?;
        }
    }
    Ok(())
}

/// What one side of a `hierarchy.split` creates for each occurrence.
///
/// "The `create` method defines the element to create. In the case of a
/// `resource` or `archetype`, this type is inferred by the FHIRconnect mapping
/// file it is included in"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/HierarchyMappings.adoc`,
/// §split), and its worked example writes `event` for the openEHR side. The
/// specification enumerates no others and no schema constrains the key, so the
/// closed set is FerroBRIDGE's own: no specification governs this: our own
/// design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Create {
    /// A FHIR resource of the type the mapping file names.
    Resource,
    /// An openEHR `EVENT` of the archetype the mapping file names.
    Event,
    /// An openEHR archetype root of the type the mapping file names.
    Archetype,
}

impl Create {
    /// Returns the spelling a mapping file writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::Event => "event",
            Self::Archetype => "archetype",
        }
    }

    /// Every spelling the key admits, in declaration order.
    #[must_use]
    pub const fn admitted() -> &'static [&'static str] {
        &["resource", "event", "archetype"]
    }
}

impl fmt::Display for Create {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Create {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "resource" => Ok(Self::Resource),
            "event" => Ok(Self::Event),
            "archetype" => Ok(Self::Archetype),
            _ => Err(()),
        }
    }
}

/// One side of a compiled `hierarchy.split`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    create: Option<Create>,
    path: Option<Target>,
    unique: Vec<Target>,
}

impl Split {
    /// Assembles one side of a split.
    #[must_use]
    pub const fn new(create: Option<Create>, path: Option<Target>, unique: Vec<Target>) -> Self {
        Self {
            create,
            path,
            unique,
        }
    }

    /// Returns the element the split creates.
    #[must_use]
    pub const fn create(&self) -> Option<Create> {
        self.create
    }

    /// Returns the resolved path the element is created at.
    #[must_use]
    pub const fn path(&self) -> Option<&Target> {
        self.path.as_ref()
    }

    /// Returns the resolved paths whose distinct values trigger a split.
    #[must_use]
    pub fn unique(&self) -> &[Target] {
        &self.unique
    }
}

/// The compiled `preprocessor` of one model or extension mapping file.
///
/// A preprocessor condition "defines that the mapping file is only executed if
/// the given condition is met"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §Conditions in the preprocessor), so the gate belongs to the file that
/// wrote it. One program carries one of these per contributing file, which is
/// how a slotted model mapping and an extension keep their own gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preprocessor {
    model: MappingName,
    fhir_condition: Option<Condition>,
    openehr_condition: Option<Condition>,
    hierarchy: Option<Hierarchy>,
}

impl Preprocessor {
    /// Assembles the compiled preprocessor of one file.
    #[must_use]
    pub const fn new(
        model: MappingName,
        fhir_condition: Option<Condition>,
        openehr_condition: Option<Condition>,
        hierarchy: Option<Hierarchy>,
    ) -> Self {
        Self {
            model,
            fhir_condition,
            openehr_condition,
            hierarchy,
        }
    }

    /// Returns the `metadata.name` of the file the preprocessor came from.
    #[must_use]
    pub const fn model(&self) -> &MappingName {
        &self.model
    }

    /// Returns the gate the FHIR to openEHR direction evaluates.
    #[must_use]
    pub const fn fhir_condition(&self) -> Option<&Condition> {
        self.fhir_condition.as_ref()
    }

    /// Returns the gate the openEHR to FHIR direction evaluates.
    #[must_use]
    pub const fn openehr_condition(&self) -> Option<&Condition> {
        self.openehr_condition.as_ref()
    }

    /// Returns the hierarchy realignment the file writes.
    #[must_use]
    pub const fn hierarchy(&self) -> Option<&Hierarchy> {
        self.hierarchy.as_ref()
    }

    /// Whether the file's preprocessor carries nothing the engine runs.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.fhir_condition.is_none()
            && self.openehr_condition.is_none()
            && self.hierarchy.is_none()
    }
}

/// The compiled `preprocessor.hierarchy` of one mapping file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hierarchy {
    fhir: Option<FhirTarget>,
    openehr: Option<OpenehrTarget>,
    split_fhir: Option<Split>,
    split_openehr: Option<Split>,
}

impl Hierarchy {
    /// Assembles a compiled hierarchy mapping.
    #[must_use]
    pub const fn new(
        fhir: Option<FhirTarget>,
        openehr: Option<OpenehrTarget>,
        split_fhir: Option<Split>,
        split_openehr: Option<Split>,
    ) -> Self {
        Self {
            fhir,
            openehr,
            split_fhir,
            split_openehr,
        }
    }

    /// Returns the FHIR side of the iterated path.
    #[must_use]
    pub const fn fhir(&self) -> Option<&FhirTarget> {
        self.fhir.as_ref()
    }

    /// Returns the openEHR side of the iterated path.
    #[must_use]
    pub const fn openehr(&self) -> Option<&OpenehrTarget> {
        self.openehr.as_ref()
    }

    /// Returns the split that creates FHIR resources.
    #[must_use]
    pub const fn split_fhir(&self) -> Option<&Split> {
        self.split_fhir.as_ref()
    }

    /// Returns the split that creates openEHR elements.
    #[must_use]
    pub const fn split_openehr(&self) -> Option<&Split> {
        self.split_openehr.as_ref()
    }
}

/// One compiled context: the immutable program the interpreter runs.
///
/// Nothing mutates a program after [`crate::resolve::compile`] returns it: the
/// fields are private, every method takes `&self`, and the program is shared
/// behind an `Arc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    context: MappingName,
    profile: ProfileBinding,
    template: TemplateBinding,
    resource: ResourceType,
    start: MappingName,
    models: Vec<ModelBinding>,
    operational: Vec<MappingName>,
    preprocessors: Vec<Preprocessor>,
    mappings: Vec<Mapping>,
}

/// Everything a program carries, as the compiler assembles it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramParts {
    /// The `metadata.name` of the context mapping.
    pub context: MappingName,
    /// The profile the context maps.
    pub profile: ProfileBinding,
    /// The template the context maps.
    pub template: TemplateBinding,
    /// The resource type the start model mapping names.
    pub resource: ResourceType,
    /// The model mapping the context starts at.
    pub start: MappingName,
    /// Every model mapping compiled, in first-use order.
    pub models: Vec<ModelBinding>,
    /// The operational mappings the context declares.
    pub operational: Vec<MappingName>,
    /// The compiled preprocessor of the start model mapping and of every
    /// extension applied to it, in application order.
    pub preprocessors: Vec<Preprocessor>,
    /// The compiled mappings, in execution order.
    pub mappings: Vec<Mapping>,
}

impl Program {
    /// Assembles a program.
    #[must_use]
    pub fn new(parts: ProgramParts) -> Self {
        Self {
            context: parts.context,
            profile: parts.profile,
            template: parts.template,
            resource: parts.resource,
            start: parts.start,
            models: parts.models,
            operational: parts.operational,
            preprocessors: parts.preprocessors,
            mappings: parts.mappings,
        }
    }

    /// Returns the `metadata.name` of the context mapping.
    #[must_use]
    pub const fn context(&self) -> &MappingName {
        &self.context
    }

    /// Returns the profile the context maps.
    #[must_use]
    pub const fn profile(&self) -> &ProfileBinding {
        &self.profile
    }

    /// Returns the template the context maps.
    #[must_use]
    pub const fn template(&self) -> &TemplateBinding {
        &self.template
    }

    /// Returns the resource type the program reads and writes.
    #[must_use]
    pub const fn resource(&self) -> &ResourceType {
        &self.resource
    }

    /// Returns the model mapping the context starts at.
    #[must_use]
    pub const fn start(&self) -> &MappingName {
        &self.start
    }

    /// Returns every model mapping the program compiled, in first-use order.
    #[must_use]
    pub fn models(&self) -> &[ModelBinding] {
        &self.models
    }

    /// Returns the operational mappings the context declares.
    #[must_use]
    pub fn operational(&self) -> &[MappingName] {
        &self.operational
    }

    /// Returns the preprocessor of every file that gates this program, in
    /// application order: the start model mapping first, then its extensions.
    #[must_use]
    pub fn preprocessors(&self) -> &[Preprocessor] {
        &self.preprocessors
    }

    /// Returns the preprocessor of the start model mapping.
    #[must_use]
    pub fn preprocessor(&self) -> Option<&Preprocessor> {
        self.preprocessors
            .iter()
            .find(|preprocessor| preprocessor.model() == &self.start)
    }

    /// Returns the compiled hierarchy mapping, when the start model has one.
    #[must_use]
    pub fn hierarchy(&self) -> Option<&Hierarchy> {
        self.preprocessor().and_then(Preprocessor::hierarchy)
    }

    /// Returns the preprocessor condition of the start model mapping the FHIR
    /// to openEHR direction evaluates.
    #[must_use]
    pub fn fhir_condition(&self) -> Option<&Condition> {
        self.preprocessor().and_then(Preprocessor::fhir_condition)
    }

    /// Returns the preprocessor condition of the start model mapping the
    /// openEHR to FHIR direction evaluates.
    #[must_use]
    pub fn openehr_condition(&self) -> Option<&Condition> {
        self.preprocessor()
            .and_then(Preprocessor::openehr_condition)
    }

    /// Returns the compiled mappings, in execution order.
    #[must_use]
    pub fn mappings(&self) -> &[Mapping] {
        &self.mappings
    }

    /// Returns the mapping method of this dotted name, at any depth.
    ///
    /// A method is addressed by its name, and "this can be also the child
    /// method. The path then would be `appendTo: parent.child`"
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
    /// §Append), which is the name a compiled mapping carries.
    #[must_use]
    pub fn mapping_named(&self, name: &str) -> Option<&Mapping> {
        walk_mappings(&self.mappings).find(|mapping| mapping.name() == name)
    }
}

/// Walks a mapping tree, parents first, through every nesting a method has.
fn walk_mappings(mappings: &[Mapping]) -> impl Iterator<Item = &Mapping> {
    let mut found: Vec<&Mapping> = Vec::new();
    let mut stack: Vec<&Mapping> = mappings.iter().rev().collect();
    while let Some(mapping) = stack.pop() {
        found.push(mapping);
        let nested = match *mapping.method() {
            Method::Reference { ref mappings, .. } | Method::Slot { ref mappings, .. } => {
                mappings.as_slice()
            }
            Method::Value
            | Method::Link { .. }
            | Method::Programmed { .. }
            | Method::Participation { .. } => &[][..],
        };
        for child in nested.iter().chain(mapping.followed_by()).rev() {
            stack.push(child);
        }
    }
    found.into_iter()
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "program {}", self.context)?;
        writeln!(f, "  profile {}", self.profile)?;
        writeln!(f, "  template {}", self.template)?;
        writeln!(f, "  resource {}", self.resource)?;
        writeln!(f, "  start {}", self.start)?;
        for model in &self.models {
            writeln!(
                f,
                "  model {} version {} archetype {} revision {}",
                model.name(),
                model.version(),
                model
                    .archetype()
                    .map_or_else(|| String::from("-"), ArchetypeId::to_string),
                model.revision()
            )?;
            for extension in model.extensions() {
                writeln!(f, "    extension {extension}")?;
            }
        }
        for operational in &self.operational {
            writeln!(f, "  operational {operational}")?;
        }
        for preprocessor in &self.preprocessors {
            render_preprocessor(f, "  ", preprocessor)?;
        }
        for mapping in &self.mappings {
            mapping.render(f, 1)?;
        }
        Ok(())
    }
}
