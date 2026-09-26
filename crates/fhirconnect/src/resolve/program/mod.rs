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

pub mod binding;
pub mod condition;
pub mod hierarchy;
pub mod manual;
pub mod mapping;
pub mod target;

use core::fmt;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::header::archetype::ArchetypeId;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::paths::FlatId;

use crate::resolve::program::binding::ModelBinding;
use crate::resolve::program::binding::ProfileBinding;
use crate::resolve::program::binding::ResourceType;
use crate::resolve::program::binding::TemplateBinding;
use crate::resolve::program::condition::Condition;
use crate::resolve::program::hierarchy::Hierarchy;
use crate::resolve::program::hierarchy::Preprocessor;
use crate::resolve::program::mapping::Mapping;
use crate::resolve::program::mapping::Method;
use crate::resolve::program::mapping::render_preprocessor;

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
    warnings: Vec<Diagnostic>,
    root_axes: Vec<FlatId>,
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
            warnings: Vec::new(),
            root_axes: Vec::new(),
        }
    }

    /// Returns this program with the repeating nodes on the way to the start
    /// model's archetype root.
    #[must_use]
    pub(crate) fn with_root_axes(mut self, axes: Vec<FlatId>) -> Self {
        self.root_axes = axes;
        self
    }

    /// Returns the repeating nodes on the way to the start model's archetype
    /// root, the root itself included, outermost first.
    ///
    /// `$archetype` is "the root of the archetype" as `$resource` is the root
    /// of the resource
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`,
    /// §`$archetype` and `$resource`), so a run into openEHR writes one
    /// resource into one instance of the root and binds these axes to it
    /// before the first mapping runs.
    #[must_use]
    pub fn root_axes(&self) -> &[FlatId] {
        &self.root_axes
    }

    /// Returns this program with the warnings its compilation raised.
    #[must_use]
    pub(crate) fn with_warnings(mut self, warnings: Vec<Diagnostic>) -> Self {
        self.warnings = warnings;
        self
    }

    /// Returns the warnings the compilation raised: what it accepted and found
    /// worth reporting, in the order it found them.
    #[must_use]
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
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
        if !self.root_axes.is_empty() {
            let axes: Vec<&str> = self.root_axes.iter().map(FlatId::as_str).collect();
            writeln!(f, "  root axes [{}]", axes.join(", "))?;
        }
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
        for warning in &self.warnings {
            writeln!(f, "  warning {}: {}", warning.code(), warning.message())?;
        }
        Ok(())
    }
}
