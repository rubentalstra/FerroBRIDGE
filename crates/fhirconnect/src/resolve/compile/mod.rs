// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Compiling one context mapping into one immutable program.
//!
//! The compiler is a pure function of the loaded set, the operational
//! template, the FHIR element table and the registry of programmed functions.
//! It selects the start model mapping, applies the extensions the context
//! declares, resolves every path on both sides, and refuses on every
//! disagreement it finds rather than on the first.
//!
//! Two anchors travel with the walk. On the FHIR side `$fhirRoot` is the
//! expression of the enclosing mapping step and `$resource` the root of the
//! resource in play; on the openEHR side `$openehrRoot` is the path of the
//! enclosing step, `$archetype` the path the current model mapping is rooted
//! at and `$composition` the root of the template
//! (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`).

pub mod derived;
pub mod fhir;
pub mod header;
pub mod hierarchy;
pub mod mapping;
pub mod method;
pub mod openehr;

use std::path::PathBuf;
use std::sync::Arc;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::diagnostic::Severity;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_rm::v1_2::paths::RmPath;

use crate::model::ast::ModelMappingFile;
use crate::model::ast::keyword::DataType;
use crate::model::ast::keyword::Direction;
use crate::model::load::MappingSet;
use crate::model::semantic::MappingCodeRegistry;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::Program;
use crate::resolve::program::ProgramParts;
use crate::resolve::program::binding::ModelBinding;
use crate::resolve::program::binding::ResourceType;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;
use crate::tree::element::Table;
use crate::tree::path::FhirPath;

use crate::resolve::compile::fhir::root_expression;
use crate::resolve::compile::openehr::compaction_map;
use crate::resolve::compile::openehr::interior_paths;
use crate::resolve::compile::openehr::occurrences;

/// Compiles the context `context` names into one immutable program.
///
/// The result is shared behind an `Arc` and never changes; compiling the same
/// inputs twice yields the same program.
///
/// # Errors
///
/// Returns every refusal the compilation raised: an unknown or dangling
/// reference, a collision between two extensions, a version selector the files
/// and the template disagree on, and every path that does not resolve against
/// the template or the element table.
pub fn compile(
    set: &MappingSet,
    context: &MappingName,
    template: &WebTemplateIndex,
    table: &dyn Table,
    codes: &dyn MappingCodeRegistry,
) -> Result<Arc<Program>, Vec<Diagnostic>> {
    let mut compiler = Compiler::new(set, template, table, codes);
    let program = compiler.run(context);
    let refused = compiler
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity() == Severity::Error);
    match program {
        Some(program) if !refused => Ok(Arc::new(program.with_warnings(compiler.diagnostics))),
        _ => Err(compiler.diagnostics),
    }
}

/// The anchors and the model in play while one mapping method compiles.
#[derive(Debug, Clone)]
struct Scope {
    /// The resource type `$resource` names.
    resource: ResourceType,
    /// The expression `$fhirRoot` names, rooted at `$resource`.
    fhir: FhirPath,
    /// The path `$archetype` names.
    archetype: RmPath,
    /// The path `$openehrRoot` names.
    openehr: RmPath,
    /// The direction the enclosing file or method pinned.
    direction: Option<Direction>,
    /// The resources a `^` climbs into once it leaves this one, innermost
    /// first.
    enclosing: Vec<Enclosing>,
    /// The file whose `spec` keys the enclosing methods inherit.
    file: PathBuf,
    /// The `spec.conceptmap` of that file, when it writes one.
    conceptmap: Option<String>,
    /// The dotted name of the enclosing method, empty at the top level.
    prefix: String,
    /// The model mappings on the slot chain, the start first.
    chain: Vec<MappingName>,
}

impl Scope {
    /// Returns the dotted name of a method called `name` in this scope.
    fn name_of(&self, name: &str) -> String {
        if self.prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{}.{name}", self.prefix)
        }
    }
}

/// One resource a `reference` mapping was written inside.
///
/// A `^` that leaves the resource a `reference` initializes carries on in the
/// expression that named the reference, which is what the worked `^^` example
/// does
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/path_operators.adoc`,
/// §Recurrence and parent elements).
#[derive(Debug, Clone)]
struct Enclosing {
    /// The resource type `$resource` named there.
    resource: ResourceType,
    /// The expression that named the reference, rooted at that `$resource`.
    fhir: FhirPath,
}

/// What a compiled FHIR expression is used for.
///
/// A mapping writes the FHIR side unless it is pinned to `fhir->openehr`
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Direction),
/// so a filtering step there is a refusal. A condition is "applied on the
/// input data" and its own `targetAttribute` example is
/// `$resource.identifier.where(type.coding.code="room")`
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`), and a
/// `hierarchy` `with` and a `split.unique` are "used as an indicator" rather
/// than transformed
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/HierarchyMappings.adoc`,
/// §Hierarchy and unique values), so those sites only ever read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Site {
    /// The expression is written through, unless `direction` pins the mapping
    /// to `fhir->openehr`.
    Write(Option<Direction>),
    /// The expression is only ever read, so a step that filters is legitimate.
    ReadOnly,
}

/// What the compiler reads of one mapping to derive its conversion.
#[derive(Debug, Clone, Copy)]
struct Derivation<'target> {
    fhir: Option<&'target FhirTarget>,
    openehr: Option<&'target OpenehrTarget>,
    data_type: Option<DataType>,
    direction: Option<Direction>,
    /// The mapping maps its two paths and nothing else: no `manual` entry and
    /// no other mapping method.
    plain: bool,
    children: bool,
}

/// The compiler's state for one run.
struct Compiler<'a> {
    set: &'a MappingSet,
    template: &'a WebTemplateIndex,
    table: &'a dyn Table,
    codes: &'a dyn MappingCodeRegistry,
    compacted: Vec<(RmPath, &'a ResolvedNode)>,
    interior: Vec<RmPath>,
    extensions: Vec<&'a ModelMappingFile>,
    diagnostics: Vec<Diagnostic>,
    models: Vec<ModelBinding>,
}

impl<'a> Compiler<'a> {
    /// Creates a compiler over one loaded set and one template.
    fn new(
        set: &'a MappingSet,
        template: &'a WebTemplateIndex,
        table: &'a dyn Table,
        codes: &'a dyn MappingCodeRegistry,
    ) -> Self {
        Self {
            set,
            template,
            table,
            codes,
            compacted: compaction_map(template),
            interior: interior_paths(template),
            extensions: Vec::new(),
            diagnostics: Vec::new(),
            models: Vec::new(),
        }
    }

    /// Compiles one context, or returns `None` when it cannot start.
    fn run(&mut self, context: &MappingName) -> Option<Program> {
        let Some(file) = self.set.context(context) else {
            self.diagnostics.push(
                Diagnostic::error(
                    PathBuf::new(),
                    ResolveCode::UnknownContext.into(),
                    format!("`{context}` is no context mapping of the loaded set"),
                )
                .with_mapping_name(context.clone()),
            );
            return None;
        };
        let declaration = file.context();
        let profile = Self::profile_of(file);
        let template = self.template_of(file);
        let start_name = declaration.start.value();
        let start = self.set.model(start_name).or_else(|| {
            self.diagnostics.push(diagnostic(
                file.file(),
                context,
                ResolveCode::UnknownContextReference,
                declaration.start.position(),
                &ModelPath::root().field("context").field("start"),
                format!("`{start_name}` is no model mapping of the loaded set"),
            ));
            None
        })?;
        self.extensions = self.extensions_of(file);
        let resource = self.resource_of(start)?;
        let archetype = self.archetype_root(start)?;
        let root = self
            .template
            .nodes()
            .find(|node| *node.rm_path() == archetype);
        let root_axes = match root.map(|node| occurrences(self.template, node)) {
            Some(Ok(axes)) => axes,
            None => Vec::new(),
            Some(Err(error)) => {
                self.diagnostics.push(diagnostic(
                    start.file(),
                    start_name,
                    ResolveCode::UnresolvedArchetypeRoot,
                    start.spec().position,
                    &ModelPath::root().field("spec").field("openEhrConfig"),
                    format!("the repeating nodes above `{archetype}` do not resolve: {error}"),
                ));
                return None;
            }
        };
        let scope = Scope {
            resource: resource.clone(),
            fhir: root_expression(),
            archetype: archetype.clone(),
            openehr: archetype,
            direction: start.spec().unidirectional.as_ref().map(|d| *d.value()),
            enclosing: Vec::new(),
            file: start.file().to_path_buf(),
            conceptmap: start
                .spec()
                .conceptmap
                .as_ref()
                .map(|url| url.value().clone()),
            prefix: String::new(),
            chain: vec![start_name.clone()],
        };
        let merged = self.merge(start);
        let mappings = self.mappings(&merged.mappings, &scope);
        let preprocessors = self.preprocessors(start, &scope);
        self.unreached_extensions(file);
        Some(
            Program::new(ProgramParts {
                context: context.clone(),
                profile,
                template,
                resource,
                start: start_name.clone(),
                models: core::mem::take(&mut self.models),
                operational: declaration
                    .operational
                    .iter()
                    .map(|name| name.value().clone())
                    .collect(),
                preprocessors,
                mappings,
            })
            .with_root_axes(root_axes),
        )
    }
}
