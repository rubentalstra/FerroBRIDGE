// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One resolved mapping, its concept method and the pair a mapping with no
//! `type` converts through.

use core::fmt;

use openehr_mapping_core::header::metadata::MappingName;

use crate::model::ast::keyword::DataType;
use crate::model::ast::keyword::Direction;

use crate::resolve::program::binding::ResourceType;
use crate::resolve::program::condition::Condition;
use crate::resolve::program::condition::render_condition;
use crate::resolve::program::hierarchy::Create;
use crate::resolve::program::hierarchy::Preprocessor;
use crate::resolve::program::manual::Manual;
use crate::resolve::program::manual::render_manual;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;

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

/// The alternative of a choice element a mapping writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alternative {
    code: &'static str,
    target: FhirTarget,
}

impl Alternative {
    /// Pairs the FHIR type code of the alternative with the target that
    /// writes it.
    #[must_use]
    pub const fn new(code: &'static str, target: FhirTarget) -> Self {
        Self { code, target }
    }

    /// Returns the FHIR type code of the alternative.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the target that writes the alternative, the choice resolved to
    /// it.
    #[must_use]
    pub const fn target(&self) -> &FhirTarget {
        &self.target
    }
}

/// What the data-type cell of a mapping with no `type` key converts, as the
/// compiler derived it from the two sides.
///
/// The `type` key is deprecated because the type "is derivable from the
/// instances of FHIR and openEHR"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/data-mappings.adoc`,
/// §Deprecated); [`crate::resolve::derive`] holds the rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Derived {
    /// The FHIR type code the cell converts through.
    Element(&'static str),
    /// A choice element no type filter resolved, read as the alternative the
    /// document carries and written as the one the node's class selects.
    ///
    /// An `Extension` against a data value converts through its `value[x]`,
    /// the one element of an extension that carries data
    /// (<https://hl7.org/fhir/R4/extensibility.html>), so `read` is that
    /// choice below the mapping's own element.
    Choice {
        /// The choice element the value is read at.
        read: FhirTarget,
        /// The alternative written, `None` for a mapping that never writes
        /// FHIR.
        write: Option<Alternative>,
    },
    /// The node holds other nodes, so the mapping anchors its children and
    /// converts nothing, as `type: NONE` does.
    Anchor,
    /// A choice element the `type` key resolves, read and written as the
    /// alternative of that type.
    Declared(Alternative),
}

impl fmt::Display for Derived {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Element(code) => write!(f, "{code}"),
            Self::Choice {
                ref read,
                ref write,
            } => {
                write!(f, "choice read {read}")?;
                if let Some(written) = write.as_ref() {
                    write!(f, " write {} as {}", written.code, written.target)?;
                }
                Ok(())
            }
            Self::Anchor => f.write_str("anchor"),
            Self::Declared(ref written) => {
                write!(f, "{} as {}", written.code, written.target)
            }
        }
    }
}

/// One compiled mapping method, with everything the interpreter needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    name: String,
    model: MappingName,
    fhir: Option<FhirTarget>,
    openehr: Option<OpenehrTarget>,
    data_type: Option<DataType>,
    derived: Option<Derived>,
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
    /// The conversion the compiler derived for a mapping with no `type` key.
    pub derived: Option<Derived>,
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
            derived: parts.derived,
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

    /// Returns the conversion the compiler derived, for a mapping with no
    /// `type` key.
    #[must_use]
    pub const fn derived(&self) -> Option<&Derived> {
        self.derived.as_ref()
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
    pub(super) fn render(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
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
        if let Some(ref derived) = self.derived {
            writeln!(f, "{inner}derived {derived}")?;
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
pub(super) fn render_preprocessor(
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
