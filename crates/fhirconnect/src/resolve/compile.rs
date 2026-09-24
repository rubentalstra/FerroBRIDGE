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

use core::str::FromStr;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;

use fhir_types::schema::ValueKind;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::diagnostic::Severity;
use openehr_mapping_core::header::ArchetypeId;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::header::MappingType;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::index::archetype_release_version;
use openehr_mapping_core::index::node_id_matches;
use openehr_mapping_core::path::MappingPath;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;
use openehr_mapping_core::template::PathError;
use openehr_rm::v1_2::model as rm_model;
use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::RmPath;

use crate::model::ast::Condition;
use crate::model::ast::ContextMappingFile;
use crate::model::ast::DataType;
use crate::model::ast::Direction;
use crate::model::ast::ManualEntry;
use crate::model::ast::ModelMappingFile;
use crate::model::ast::SplitTarget;
use crate::model::ast::Variable;
use crate::model::ast::With;
use crate::model::load::MappingSet;
use crate::model::semantic::MappingCodeRegistry;
use crate::resolve::derive;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::Merged;
use crate::resolve::extensions::Node;
use crate::resolve::extensions::apply;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::Alternative;
use crate::resolve::program::Attachment;
use crate::resolve::program::Condition as CompiledCondition;
use crate::resolve::program::ConditionParts;
use crate::resolve::program::Create;
use crate::resolve::program::Derived;
use crate::resolve::program::FhirTarget;
use crate::resolve::program::Hierarchy;
use crate::resolve::program::Manual;
use crate::resolve::program::ManualPath;
use crate::resolve::program::ManualValue;
use crate::resolve::program::Mapping;
use crate::resolve::program::MappingParts;
use crate::resolve::program::Method;
use crate::resolve::program::ModelBinding;
use crate::resolve::program::OpenehrTarget;
use crate::resolve::program::Pin;
use crate::resolve::program::Preprocessor;
use crate::resolve::program::ProfileBinding;
use crate::resolve::program::ProfileUrl;
use crate::resolve::program::Program;
use crate::resolve::program::ProgramParts;
use crate::resolve::program::ResourceType;
use crate::resolve::program::Split;
use crate::resolve::program::Target;
use crate::resolve::program::TemplateBinding;
use crate::resolve::program::TemplateId;
use crate::tree::element::Location;
use crate::tree::element::Move;
use crate::tree::element::Table;
use crate::tree::element::resolve as resolve_element;
use crate::tree::path::FhirPath;
use crate::tree::path::Writability;

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

/// The two resolved sides of a mapping with no `type` key, as its conversion
/// is derived from them.
#[derive(Debug, Clone, Copy)]
struct Sides<'target> {
    fhir: &'target FhirTarget,
    openehr: &'target OpenehrTarget,
    direction: Option<Direction>,
    children: bool,
}

/// The FHIR type every extension is (<https://hl7.org/fhir/R4/extensibility.html>).
const EXTENSION_TYPE: &str = "Extension";

/// The element of an extension that carries its value, `Extension.value[x]`.
const EXTENSION_VALUE: &str = "value";

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

    /// Reports every listed extension whose target model the program never
    /// reached.
    ///
    /// An extension applies when the model mapping it extends is merged, so
    /// one whose model no slot of the chain reaches changes nothing. The
    /// context still compiles, and the listing is reported as a warning
    /// because an author who lists an extension expects it to act. No
    /// specification governs this: our own design.
    fn unreached_extensions(&mut self, file: &ContextMappingFile) {
        let owner = file.header().name().value();
        let path = ModelPath::root().field("context").field("extensions");
        for (index, declared) in file.context().extensions.iter().enumerate() {
            let Some(extension) = self.set.model(declared.value()) else {
                continue;
            };
            let Some(extends) = extension.spec().extends.as_ref() else {
                continue;
            };
            let applied = self.models.iter().any(|model| {
                model
                    .extensions()
                    .iter()
                    .any(|name| name == declared.value())
            });
            if applied {
                continue;
            }
            self.diagnostics.push(
                Diagnostic::warning(
                    file.file().to_path_buf(),
                    ResolveCode::UnreachedExtension.into(),
                    format!(
                        "`{}` extends `{}`, which no mapping of this context reaches, so the \
                         extension changes nothing",
                        declared.value(),
                        extends.value()
                    ),
                )
                .with_position(declared.position())
                .with_mapping_name(owner.clone())
                .with_model_path(path.index(index)),
            );
        }
    }

    /// Reads the profile the context pins.
    fn profile_of(file: &ContextMappingFile) -> ProfileBinding {
        let profile = &file.context().profile;
        ProfileBinding::new(
            ProfileUrl::new(
                profile
                    .url
                    .as_ref()
                    .map_or("", |url| url.value().as_str())
                    .to_owned(),
            ),
            Pin::new(profile.version.as_ref().map(|v| v.value().as_str())),
        )
    }

    /// Reads the template the context pins and checks it against the template
    /// the program compiles against.
    fn template_of(&mut self, file: &ContextMappingFile) -> TemplateBinding {
        let declared = &file.context().template;
        let name = file.header().name().value();
        let path = ModelPath::root().field("context").field("template");
        let id = declared.id.as_ref().map_or_else(
            || self.template.template_id().to_owned(),
            |id| {
                if id.value() != self.template.template_id() {
                    self.diagnostics.push(diagnostic(
                        file.file(),
                        name,
                        ResolveCode::TemplateIdMismatch,
                        id.position(),
                        &path.field("id"),
                        format!(
                            "the context names the template `{}` and the program compiles against \
                             `{}`",
                            id.value(),
                            self.template.template_id()
                        ),
                    ));
                }
                id.value().clone()
            },
        );
        let sem_ver = declared.sem_ver.as_ref();
        if let (Some(declared_version), Some(carried)) = (sem_ver, self.template.sem_ver())
            && declared_version.value() != carried
        {
            self.diagnostics.push(diagnostic(
                file.file(),
                name,
                ResolveCode::TemplateSemVerMismatch,
                declared_version.position(),
                &path.field("sem_ver"),
                format!(
                    "the context pins `template.sem_ver` `{}` and the template carries `{carried}`",
                    declared_version.value()
                ),
            ));
        }
        TemplateBinding::new(
            TemplateId::new(id),
            Pin::new(sem_ver.map(|version| version.value().as_str())),
            self.template.generation(),
        )
    }

    /// Reads the extension files the context declares, in declaration order.
    fn extensions_of(&mut self, file: &ContextMappingFile) -> Vec<&'a ModelMappingFile> {
        let name = file.header().name().value();
        let path = ModelPath::root().field("context").field("extensions");
        let mut found = Vec::new();
        for (index, declared) in file.context().extensions.iter().enumerate() {
            let at = path.index(index);
            let Some(extension) = self.set.model(declared.value()) else {
                self.diagnostics.push(diagnostic(
                    file.file(),
                    name,
                    ResolveCode::UnknownContextReference,
                    declared.position(),
                    &at,
                    format!(
                        "`{}` is no model mapping of the loaded set",
                        declared.value()
                    ),
                ));
                continue;
            };
            let is_extension = extension.header().mapping_type().map(|kind| *kind.value())
                == Some(MappingType::Extension);
            if !is_extension {
                self.diagnostics.push(diagnostic(
                    file.file(),
                    name,
                    ResolveCode::NotAnExtension,
                    declared.position(),
                    &at,
                    format!(
                        "`{}` is loaded as a `{}` file, and `context.extensions` names extensions",
                        declared.value(),
                        extension
                            .header()
                            .mapping_type()
                            .map_or("model", |kind| kind.value().as_str())
                    ),
                ));
                continue;
            }
            if extension.spec().extends.is_none() {
                self.diagnostics.push(diagnostic(
                    extension.file(),
                    extension.header().name().value(),
                    ResolveCode::ExtensionWithoutTarget,
                    extension.spec().position,
                    &ModelPath::root().field("spec").field("extends"),
                    format!(
                        "`{}` extends no model mapping, so nothing says what it applies to",
                        declared.value()
                    ),
                ));
                continue;
            }
            found.push(extension);
        }
        found
    }

    /// Returns the extensions that extend `model`, in declaration order.
    fn extensions_for(&self, model: &'a ModelMappingFile) -> Vec<&'a ModelMappingFile> {
        let name = model.header().name().value();
        self.extensions
            .iter()
            .copied()
            .filter(|extension| {
                extension
                    .spec()
                    .extends
                    .as_ref()
                    .is_some_and(|extends| extends.value() == name)
            })
            .collect()
    }

    /// Applies the extensions that extend `model`, in declaration order.
    fn merge(&mut self, model: &'a ModelMappingFile) -> Merged<'a> {
        let name = model.header().name().value();
        let mine = self.extensions_for(model);
        let merged = apply(model, &mine, &mut self.diagnostics);
        if !self.models.iter().any(|model| model.name() == name) {
            self.models.push(ModelBinding::new(
                name.clone(),
                model.header().version().value().clone(),
                archetype_of(model).cloned(),
                Pin::new(
                    model
                        .header()
                        .revision()
                        .and_then(openehr_mapping_core::value::PositionedValue::as_text),
                ),
                merged.applied.clone(),
            ));
        }
        merged
    }

    /// Reads the resource type the start model mapping names.
    ///
    /// FHIRconnect calls `fhirConfig.structureDefinition` information for the
    /// user (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`,
    /// §Spec) and names no other source for the resource type, so the compiler
    /// reads the last segment of that canonical URL, which is the resource
    /// name for a base resource
    /// (<https://hl7.org/fhir/R4/structuredefinition.html>). Requiring the key
    /// and deriving the resource type from it is FerroBRIDGE's own decision:
    /// no specification governs this: our own design.
    fn resource_of(&mut self, model: &ModelMappingFile) -> Option<ResourceType> {
        let name = model.header().name().value();
        let path = ModelPath::root()
            .field("spec")
            .field("fhirConfig")
            .field("structureDefinition");
        let Some(ref definition) = model.spec().structure_definition else {
            self.diagnostics.push(diagnostic(
                model.file(),
                name,
                ResolveCode::ResourceTypeUnnamed,
                model.spec().position,
                &path,
                String::from(
                    "the start model mapping names no `fhirConfig.structureDefinition`, so no \
                     resource type is named",
                ),
            ));
            return None;
        };
        let text = definition.value();
        let candidate = text.rsplit('/').next().unwrap_or(text.as_str());
        if !self.table.is_resource(candidate) {
            self.diagnostics.push(diagnostic(
                model.file(),
                name,
                ResolveCode::UnknownResourceType,
                definition.position(),
                &path,
                format!("`{candidate}` names no resource type of this FHIR version"),
            ));
            return None;
        }
        Some(ResourceType::new(candidate))
    }

    /// Returns the path of the node the model mapping's archetype is rooted
    /// at.
    ///
    /// Every refusal here names the model mapping's own file, because the
    /// position it carries is a position in that file.
    fn archetype_root(&mut self, model: &ModelMappingFile) -> Option<RmPath> {
        let file = model.file();
        let owner = model.header().name().value();
        let path = ModelPath::root()
            .field("spec")
            .field("openEhrConfig")
            .field("archetype");
        let Some(archetype) = archetype_of(model) else {
            self.diagnostics.push(diagnostic(
                file,
                owner,
                ResolveCode::UnresolvedArchetypeRoot,
                model.spec().position,
                &path,
                format!(
                    "`{}` names no `openEhrConfig.archetype`, so `$archetype` names nothing",
                    model.header().name().value()
                ),
            ));
            return None;
        };
        let template = self.template;
        let found: Vec<&ResolvedNode> = template
            .nodes()
            .filter(|node| {
                node.node_id()
                    .is_some_and(|carried| node_id_matches(archetype.as_str(), carried))
            })
            .collect();
        if let [only] = *found.as_slice() {
            let root = only.rm_path().clone();
            self.check_revision(model, only.node_id());
            return Some(root);
        }
        self.diagnostics.push(diagnostic(
            file,
            owner,
            ResolveCode::UnresolvedArchetypeRoot,
            model.spec().position,
            &path,
            format!(
                "the template `{}` carries {} nodes for the archetype `{archetype}`",
                template.template_id(),
                found.len()
            ),
        ));
        None
    }

    /// Checks the revision a model mapping pins against the archetype the
    /// template carries.
    ///
    /// `openEhrConfig.revision` "states what revision of the archetype this
    /// mapping applies for"
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Spec),
    /// and an ADL 2 archetype identifier carries that release version below
    /// its major. A template served over the ADL 1.4 route carries the
    /// interface form alone, so there the pin is recorded and nothing is
    /// compared.
    fn check_revision(&mut self, model: &ModelMappingFile, carried: Option<&str>) {
        let Some(revision) = model.header().revision() else {
            return;
        };
        let Some(pinned) = revision.as_text() else {
            return;
        };
        let Some(release) = carried.and_then(archetype_release_version) else {
            return;
        };
        if pinned == release {
            return;
        }
        self.diagnostics.push(diagnostic(
            model.file(),
            model.header().name().value(),
            ResolveCode::ArchetypeRevisionMismatch,
            revision.position(),
            &ModelPath::root()
                .field("spec")
                .field("openEhrConfig")
                .field("revision"),
            format!(
                "`{}` pins the archetype revision `{pinned}` and the template carries \
                 `{release}`",
                model.header().name().value()
            ),
        ));
    }

    /// Compiles the preprocessor of `model` and of every extension of it.
    ///
    /// A preprocessor gates the file that wrote it
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
    /// §Conditions in the preprocessor), and an extension file is a file, so
    /// each contributes its own gate under the same anchors.
    fn preprocessors(&mut self, model: &'a ModelMappingFile, scope: &Scope) -> Vec<Preprocessor> {
        let mut found = Vec::new();
        for file in core::iter::once(model).chain(self.extensions_for(model)) {
            if let Some(compiled) = self.preprocessor(file, scope) {
                found.push(compiled);
            }
        }
        found
    }

    /// Compiles the preprocessor of one model or extension mapping file.
    fn preprocessor(&mut self, model: &'a ModelMappingFile, scope: &Scope) -> Option<Preprocessor> {
        let preprocessor = model.preprocessor()?;
        let path = ModelPath::root().field("preprocessor");
        let fhir = preprocessor.fhir_condition.as_ref().and_then(|condition| {
            self.condition(
                model,
                condition,
                scope,
                Direction::FhirToOpenehr,
                &path.field("fhirCondition"),
                scope.fhir.as_str(),
            )
        });
        let openehr = preprocessor
            .openehr_condition
            .as_ref()
            .and_then(|condition| {
                self.condition(
                    model,
                    condition,
                    scope,
                    Direction::OpenehrToFhir,
                    &path.field("openehrCondition"),
                    &scope.openehr.to_string(),
                )
            });
        let hierarchy = preprocessor.hierarchy.as_ref().map(|hierarchy| {
            let at = path.field("hierarchy");
            let with = hierarchy.with.as_ref();
            let fhir_side = with.and_then(|with| self.fhir_side(model, with, scope, &at));
            let openehr_side = with.and_then(|with| self.openehr_side(model, with, scope, &at));
            let inner = Scope {
                fhir: fhir_side
                    .as_ref()
                    .map_or_else(|| scope.fhir.clone(), |target| target.expression().clone()),
                openehr: openehr_side
                    .as_ref()
                    .map_or_else(|| scope.openehr.clone(), |target| target.path().clone()),
                ..scope.clone()
            };
            let split = hierarchy.split.as_ref();
            Hierarchy::new(
                fhir_side,
                openehr_side,
                split.and_then(|split| split.fhir.as_ref()).map(|target| {
                    self.split(
                        model,
                        target,
                        &inner,
                        true,
                        &at.field("split").field("fhir"),
                    )
                }),
                split
                    .and_then(|split| split.openehr.as_ref())
                    .map(|target| {
                        self.split(
                            model,
                            target,
                            &inner,
                            false,
                            &at.field("split").field("openehr"),
                        )
                    }),
            )
        });
        Some(Preprocessor::new(
            model.header().name().value().clone(),
            fhir,
            openehr,
            hierarchy,
        ))
    }

    /// Compiles one side of a `hierarchy.split`.
    ///
    /// The `path` of a side is a path on that side and its `unique` entries
    /// are paths on the other one, which is what the worked example writes
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/HierarchyMappings.adoc`,
    /// §Hierarchy and unique values).
    fn split(
        &mut self,
        model: &'a ModelMappingFile,
        target: &SplitTarget,
        scope: &Scope,
        creates_fhir: bool,
        path: &ModelPath,
    ) -> Split {
        let owner = model.header().name().value();
        let created = target.path.as_ref().and_then(|written| {
            let at = path.field("path");
            if creates_fhir {
                self.fhir_target(
                    model.file(),
                    owner,
                    written,
                    scope,
                    &at,
                    Site::Write(scope.direction),
                )
                .map(|target| Target::Fhir(Box::new(target)))
            } else {
                self.openehr_target(model.file(), owner, written, scope, &at)
                    .map(|target| Target::Openehr(Box::new(target)))
            }
        });
        let unique = target
            .unique
            .iter()
            .enumerate()
            .filter_map(|(index, written)| {
                let at = path.field("unique").index(index);
                if creates_fhir {
                    self.openehr_target(model.file(), owner, written, scope, &at)
                        .map(|target| Target::Openehr(Box::new(target)))
                } else {
                    self.fhir_target(model.file(), owner, written, scope, &at, Site::ReadOnly)
                        .map(|target| Target::Fhir(Box::new(target)))
                }
            })
            .collect();
        let create = target.create.as_ref().and_then(|written| {
            let Ok(create) = Create::from_str(written.value()) else {
                self.diagnostics.push(diagnostic(
                    model.file(),
                    owner,
                    ResolveCode::UnknownSplitCreate,
                    written.position(),
                    &path.field("create"),
                    format!(
                        "`{}` names no element a split creates; the engine creates `{}`",
                        written.value(),
                        Create::admitted().join("`, `")
                    ),
                ));
                return None;
            };
            Some(create)
        });
        Split::new(create, created, unique)
    }

    /// Compiles a list of mapping methods under one scope.
    ///
    /// Each node carries where it sits in the file that wrote it, so a nested
    /// refusal names its own depth rather than the index of the top-level
    /// method it hangs under.
    fn mappings(&mut self, nodes: &[Node<'a>], scope: &Scope) -> Vec<Mapping> {
        nodes
            .iter()
            .map(|node| {
                let path = node.path.clone();
                self.mapping(node, scope, &path)
            })
            .collect()
    }

    /// Compiles one mapping method and everything under it.
    fn mapping(&mut self, node: &Node<'a>, scope: &Scope, path: &ModelPath) -> Mapping {
        let file = node.origin;
        let owner = file.header().name().value();
        let method = node.mapping;
        let name = scope.name_of(node.name());
        let (inherited, conceptmap) = inherited_spec(file, scope);
        let direction = method
            .unidirectional
            .as_ref()
            .map_or(inherited, |located| Some(*located.value()));
        let with = method.with.as_ref();
        let fhir = with.and_then(|with| {
            self.fhir_side_at(
                file,
                with,
                scope,
                &path.field("with"),
                Site::Write(direction),
            )
        });
        let openehr =
            with.and_then(|with| self.openehr_side(file, with, scope, &path.field("with")));
        let inner = Scope {
            fhir: fhir
                .as_ref()
                .map_or_else(|| scope.fhir.clone(), |target| target.expression().clone()),
            openehr: openehr
                .as_ref()
                .map_or_else(|| scope.openehr.clone(), |target| target.path().clone()),
            direction,
            file: file.file().to_path_buf(),
            conceptmap: conceptmap.clone(),
            prefix: name.clone(),
            ..scope.clone()
        };
        let (fhir_condition, openehr_condition) =
            self.conditions(node, scope, fhir.as_ref(), openehr.as_ref(), path);
        let manual: Vec<Manual> = method
            .manual
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                self.manual(file, entry, &inner, &path.field("manual").index(index))
            })
            .collect();
        let method_kind = self.method(node, &inner, path);
        let followed_by = self.mappings(&node.followed_by, &inner);
        let data_type = with.and_then(|with| with.data_type.as_ref().map(|kind| *kind.value()));
        let plain =
            manual.is_empty() && matches!(method_kind, Method::Value) && !declares_method(method);
        let derived = self.derived_of(
            file,
            &Derivation {
                fhir: fhir.as_ref(),
                openehr: openehr.as_ref(),
                data_type,
                direction,
                plain,
                children: !followed_by.is_empty(),
            },
            method.position,
            &path.field("with"),
        );
        if let (Some(_), Some(target)) = (&fhir, &openehr)
            && plain
            && data_type != Some(DataType::None)
            && derived != Some(Derived::Anchor)
        {
            self.carried_tail(
                file,
                &name,
                target,
                false,
                method.position,
                &path.field("with").field("openehr"),
            );
        }
        Mapping::new(MappingParts {
            name,
            model: owner.clone(),
            fhir,
            openehr,
            data_type,
            derived,
            value: with.and_then(|with| with.value.as_ref().map(|value| value.value().clone())),
            direction,
            fhir_condition,
            openehr_condition,
            manual,
            conceptmap: method
                .conceptmap
                .as_ref()
                .map(|url| url.value().clone())
                .or(conceptmap),
            method: method_kind,
            followed_by,
        })
    }

    /// Refuses an openEHR tail no FLAT part of the node's class carries.
    ///
    /// The engine writes a tail through the FLAT parts of the node's class
    /// (`engine::rm::carried`), so a tail outside them would be merged and
    /// then dropped on the way to the wire; the crate refuses such a mapping
    /// at load, never on the request that first reaches it. A `manual` path
    /// writes a scalar, so an empty one writes the node's `value` and a
    /// family attribute is refused. No specification governs the walk below a
    /// node beyond the RM attribute model: our own design.
    fn carried_tail(
        &mut self,
        file: &'a ModelMappingFile,
        mapping: &str,
        target: &OpenehrTarget,
        manual: bool,
        position: Position,
        path: &ModelPath,
    ) {
        let segments: Vec<&str> = target
            .tail()
            .segments
            .iter()
            .map(|segment| segment.attribute.as_str())
            .collect();
        let written: &[&str] = match (segments.is_empty(), manual) {
            (true, false) => return,
            (true, true) => &["value"],
            (false, _) => &segments,
        };
        let class = target.node().rm_type();
        let carried = crate::engine::rm::carried(class, written)
            .is_some_and(|(_, held)| !manual || held != crate::engine::rm::Carried::Family);
        if carried {
            return;
        }
        self.diagnostics.push(diagnostic(
            file.file(),
            file.header().name().value(),
            ResolveCode::UncarriedTail,
            position,
            path,
            format!(
                "`{mapping}` names `{}` below the {class} node `{}`, which no FLAT part of \
                 {class} carries, so the value would never reach the wire",
                written.join("/"),
                target.node().aql_path().as_str()
            ),
        ));
    }

    /// Compiles the `fhirCondition` and the `openehrCondition` of one mapping
    /// method, each guarded by the side it reads.
    fn conditions(
        &mut self,
        node: &Node<'a>,
        scope: &Scope,
        fhir: Option<&FhirTarget>,
        openehr: Option<&OpenehrTarget>,
        path: &ModelPath,
    ) -> (Option<CompiledCondition>, Option<CompiledCondition>) {
        let file = node.origin;
        let method = node.mapping;
        let fhir_condition = method.fhir_condition.as_ref().and_then(|condition| {
            let guard = fhir.map_or(scope.fhir.as_str(), |target| target.expression().as_str());
            self.condition(
                file,
                condition,
                scope,
                Direction::FhirToOpenehr,
                &path.field("fhirCondition"),
                guard,
            )
        });
        let openehr_condition = method.openehr_condition.as_ref().and_then(|condition| {
            let guard = openehr.map_or_else(
                || scope.openehr.to_string(),
                |target| target.path().to_string(),
            );
            self.condition(
                file,
                condition,
                scope,
                Direction::OpenehrToFhir,
                &path.field("openehrCondition"),
                &guard,
            )
        });
        (fhir_condition, openehr_condition)
    }

    /// Derives what the data-type cell of a mapping converts, when the
    /// mapping leaves it to the compiler.
    ///
    /// A plain value mapping with no `type` key derives its pair from its two
    /// sides, and a mapping whose `type` key names the type of a choice element
    /// no filter resolved takes that alternative. Every other mapping carries
    /// what it wrote.
    fn derived_of(
        &mut self,
        file: &'a ModelMappingFile,
        derivation: &Derivation<'_>,
        position: Position,
        path: &ModelPath,
    ) -> Option<Derived> {
        let fhir = derivation.fhir?;
        match (derivation.openehr, derivation.data_type) {
            (Some(openehr), None) if derivation.plain => self.derive(
                file,
                Sides {
                    fhir,
                    openehr,
                    direction: derivation.direction,
                    children: derivation.children,
                },
                position,
                path,
            ),
            (_, Some(data_type))
                if derivation.plain
                    && matches!(*fhir.resolved().location(), Location::Choice(_)) =>
            {
                let code = derive::declared(data_type)?;
                self.declared_alternative(file, fhir, code, position, path)
            }
            _ => None,
        }
    }

    /// Derives the conversion of a mapping with no `type` key from its two
    /// resolved sides.
    ///
    /// A structural node anchors the children and converts nothing, as the
    /// `type: NONE` of `types-of-mappings/concept-type/FollowedBy.adoc` does,
    /// and one with no child writes nothing, which is refused. A choice
    /// element, and an `Extension` against a data value, read the alternative
    /// the document carries and write the first pair of the node's class the
    /// choice admits (`crate::resolve::derive`).
    fn derive(
        &mut self,
        file: &'a ModelMappingFile,
        sides: Sides<'_>,
        position: Position,
        path: &ModelPath,
    ) -> Option<Derived> {
        let owner = file.header().name().value();
        let Sides {
            fhir,
            openehr,
            direction,
            children,
        } = sides;
        let class = openehr.leaf_class().unwrap_or(openehr.node().rm_type());
        if derive::is_structural(class) {
            if children {
                return Some(Derived::Anchor);
            }
            // NOTE: no specification governs this: our own design, the mapping is kept
            // with no derived pair so a run that carries its element refuses with no cell.
            self.diagnostics.push(
                Diagnostic::warning(
                    file.file().to_path_buf(),
                    ResolveCode::AnchorWithoutChildren.into(),
                    format!(
                        "`{}` names the {class} node `{}`, which holds other nodes, and no \
                         `type` or child says what to write into it, so a run that carries \
                         the element refuses",
                        fhir.expression(),
                        openehr.path()
                    ),
                )
                .with_position(position)
                .with_mapping_name(owner.clone())
                .with_model_path(path.clone()),
            );
            return None;
        }
        let read = match *fhir.resolved().location() {
            Location::Choice(_) => fhir.clone(),
            Location::Complex(schema)
                if schema.name == EXTENSION_TYPE && !derive::pairs(class).is_empty() =>
            {
                self.derived_target(file, fhir, EXTENSION_VALUE, position, path)?
            }
            Location::Complex(_) | Location::Primitive(_) | Location::Attribute => {
                let code = fhir.resolved().type_code()?;
                let text = matches!(
                    *fhir.resolved().location(),
                    Location::Primitive(ValueKind::Text) | Location::Attribute
                );
                return Some(Derived::Element(derive::element(class, code, text)));
            }
            Location::PrimitiveElement | Location::Resource | Location::Deferred => return None,
        };
        if direction == Some(Direction::FhirToOpenehr) {
            return Some(Derived::Choice { read, write: None });
        }
        let variants = match read.resolved().moves().last() {
            Some(Move::Choice { variants, .. }) => *variants,
            _ => &[],
        };
        let Some(suffix) = derive::alternative(class, variants) else {
            self.diagnostics.push(diagnostic(
                file.file(),
                owner,
                ResolveCode::UnderivedAlternative,
                position,
                path,
                format!(
                    "`{}` is written from the {class} node `{}`, and the choice admits none of \
                     the types {class} pairs with ({}); a `type` or an `as()` says which to write",
                    read.resolved().leaf(),
                    openehr.path(),
                    derive::pairs(class).join(", ")
                ),
            ));
            return None;
        };
        let written = self.derived_target(file, &read, &format!("as({suffix})"), position, path)?;
        let code = written.resolved().type_code().unwrap_or(suffix);
        Some(Derived::Choice {
            read,
            write: Some(Alternative::new(code, written)),
        })
    }

    /// Resolves the alternative of a choice element the `type` key names.
    ///
    /// The key names the FHIR type of the element, the Type ID column of the
    /// table in `types-of-mappings/data-type/data-mappings.adoc` §Deprecated, so
    /// on a choice it fixes the alternative both directions read and write.
    fn declared_alternative(
        &mut self,
        file: &'a ModelMappingFile,
        fhir: &FhirTarget,
        code: &'static str,
        position: Position,
        path: &ModelPath,
    ) -> Option<Derived> {
        let variants = match fhir.resolved().moves().last() {
            Some(Move::Choice { variants, .. }) => *variants,
            _ => &[],
        };
        let Some(suffix) = derive::suffix_of(code, variants) else {
            self.diagnostics.push(diagnostic(
                file.file(),
                file.header().name().value(),
                ResolveCode::UnderivedAlternative,
                position,
                path,
                format!(
                    "the `type` of `{}` names {code}, which the choice `{}` does not admit",
                    fhir.expression(),
                    fhir.resolved().leaf()
                ),
            ));
            return None;
        };
        let written = self.derived_target(file, fhir, &format!("as({suffix})"), position, path)?;
        Some(Derived::Declared(Alternative::new(code, written)))
    }

    /// Resolves `target` extended by one step the compiler derived.
    ///
    /// The step is one the grammar admits below any element of the kind the
    /// target ends on, so a refusal is an element-table disagreement and is
    /// reported as one for a written path would be.
    fn derived_target(
        &mut self,
        file: &'a ModelMappingFile,
        target: &FhirTarget,
        step: &str,
        position: Position,
        path: &ModelPath,
    ) -> Option<FhirTarget> {
        let written = format!("{}.{step}", target.expression());
        let (code, reason) = match FhirPath::from_str(&written) {
            Ok(expression) => {
                match resolve_element(self.table, target.resolved().resource(), &expression) {
                    Ok(resolved) => return Some(FhirTarget::new(expression, resolved)),
                    Err(error) => (ResolveCode::UnknownFhirElement, error.to_string()),
                }
            }
            Err(error) => (ResolveCode::MalformedFhirPath, error.to_string()),
        };
        self.diagnostics.push(diagnostic(
            file.file(),
            file.header().name().value(),
            code,
            position,
            path,
            format!("the derived `{written}` does not resolve: {reason}"),
        ));
        None
    }

    /// Decides what a mapping method does beyond mapping its two paths.
    fn method(&mut self, node: &Node<'a>, scope: &Scope, path: &ModelPath) -> Method {
        let method = node.mapping;
        let file = node.origin;
        let owner = file.header().name().value();
        let declared = [
            method.reference.as_ref().map(|_| "reference"),
            method.slot_archetype.as_ref().map(|_| "slotArchetype"),
            method.link.as_ref().map(|_| "link"),
            method.mapping_code.as_ref().map(|_| "mappingCode"),
            method
                .participations_function
                .as_ref()
                .map(|_| "participationsFunction"),
        ];
        let written: Vec<&str> = declared.into_iter().flatten().collect();
        if written.len() > 1 {
            self.diagnostics.push(diagnostic(
                file.file(),
                owner,
                ResolveCode::ConflictingMappingMethods,
                method.position,
                path,
                format!(
                    "`{}` writes {}, and a mapping method is one of them",
                    method.name.value(),
                    written.join(", ")
                ),
            ));
        }
        if let Some(ref reference) = method.reference {
            let resource = ResourceType::new(reference.resource_type.value().clone());
            if !self.table.is_resource(resource.as_str()) {
                self.diagnostics.push(diagnostic(
                    file.file(),
                    owner,
                    ResolveCode::UnknownResourceType,
                    reference.resource_type.position(),
                    &path.field("reference").field("resourceType"),
                    format!("`{resource}` names no resource type of this FHIR version"),
                ));
                return Method::Value;
            }
            // NOTE: `path_operators.adoc` never says what `^` does at a
            // `reference` boundary, and crossing it for no `^` is the reading
            // its worked example needs (reported on issue #183).
            let mut enclosing = vec![Enclosing {
                resource: scope.resource.clone(),
                fhir: scope.fhir.clone(),
            }];
            enclosing.extend(scope.enclosing.iter().cloned());
            let inner = Scope {
                resource: resource.clone(),
                fhir: root_expression(),
                enclosing,
                ..scope.clone()
            };
            return Method::Reference {
                resource,
                mappings: self.mappings(&node.reference, &inner),
            };
        }
        if let Some(ref slot) = method.slot_archetype {
            return self.slot(file, slot, scope, path);
        }
        if let Some(ref link) = method.link {
            return Method::Link {
                meaning: link.meaning.as_ref().map(|value| value.value().clone()),
                link_type: link.link_type.as_ref().map(|value| value.value().clone()),
            };
        }
        if let Some(ref code) = method.mapping_code {
            if !self.codes.contains(code.value()) {
                self.diagnostics.push(diagnostic(
                    file.file(),
                    owner,
                    ResolveCode::UnknownContextReference,
                    code.position(),
                    &path.field("mappingCode"),
                    format!("`{}` names no function the engine registers", code.value()),
                ));
            }
            return Method::Programmed {
                code: code.value().clone(),
            };
        }
        if let Some(ref function) = method.participations_function {
            return Method::Participation {
                function: function.value().clone(),
            };
        }
        Method::Value
    }

    /// Compiles the model mapping a `slotArchetype` hands over to.
    ///
    /// The slot names the openEHR path the slotted mapping is transformed at
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/SlotArchetypes.adoc`),
    /// so `$archetype` inside it is that path. The specification says nothing
    /// about the FHIR side of the hand-over, so the slotted mapping keeps the
    /// caller's `$fhirRoot` and `$resource` stays the enclosing resource. No
    /// specification governs this: our own design.
    fn slot(
        &mut self,
        file: &'a ModelMappingFile,
        slot: &Located<MappingName>,
        scope: &Scope,
        path: &ModelPath,
    ) -> Method {
        let owner = file.header().name().value();
        let at = path.field("slotArchetype");
        let Some(slotted) = self.set.model(slot.value()) else {
            self.diagnostics.push(diagnostic(
                file.file(),
                owner,
                ResolveCode::UnknownContextReference,
                slot.position(),
                &at,
                format!("`{}` is no model mapping of the loaded set", slot.value()),
            ));
            return Method::Value;
        };
        if scope.chain.contains(slot.value()) {
            let chain: Vec<&str> = scope.chain.iter().map(MappingName::as_str).collect();
            self.diagnostics.push(diagnostic(
                file.file(),
                owner,
                ResolveCode::SlotCycle,
                slot.position(),
                &at,
                format!(
                    "`{}` slots `{}`, which is already on the chain {}",
                    owner,
                    slot.value(),
                    chain.join(" -> ")
                ),
            ));
            return Method::Value;
        }
        let archetype = archetype_of(slotted).cloned();
        if let (Some(archetype), Ok((node, _, _))) = (archetype, self.locate(&scope.openehr)) {
            match node.node_id() {
                Some(carried) if node_id_matches(archetype.as_str(), carried) => {
                    self.check_revision(slotted, Some(carried));
                }
                carried => self.diagnostics.push(diagnostic(
                    file.file(),
                    owner,
                    ResolveCode::ArchetypeMismatch,
                    slot.position(),
                    &at,
                    format!(
                        "`{}` declares the archetype `{archetype}` and the slot resolves to the \
                         node `{}`",
                        slot.value(),
                        carried.unwrap_or("with no archetype")
                    ),
                )),
            }
        }
        let mut chain = scope.chain.clone();
        chain.push(slot.value().clone());
        let inner = Scope {
            archetype: scope.openehr.clone(),
            direction: slotted
                .spec()
                .unidirectional
                .as_ref()
                .map_or(scope.direction, |located| Some(*located.value())),
            file: slotted.file().to_path_buf(),
            conceptmap: slotted
                .spec()
                .conceptmap
                .as_ref()
                .map(|url| url.value().clone()),
            prefix: String::new(),
            chain,
            ..scope.clone()
        };
        let merged = self.merge(slotted);
        Method::Slot {
            model: slot.value().clone(),
            preprocessors: self.preprocessors(slotted, &inner),
            mappings: self.mappings(&merged.mappings, &inner),
        }
    }

    /// Compiles one `manual` entry.
    fn manual(
        &mut self,
        file: &'a ModelMappingFile,
        entry: &ManualEntry,
        scope: &Scope,
        path: &ModelPath,
    ) -> Manual {
        let owner = file.header().name().value();
        let pinned = entry
            .unidirectional
            .as_ref()
            .map_or(scope.direction, |located| Some(*located.value()));
        let fhir = entry
            .fhir
            .iter()
            .enumerate()
            .filter_map(|(index, manual)| {
                let at = path.field("fhir").index(index);
                let value = self.manual_value(file, &manual.value, &at)?;
                self.fhir_target(
                    file.file(),
                    owner,
                    &manual.path,
                    scope,
                    &at,
                    Site::Write(pinned),
                )
                .map(|target| ManualPath::new(Target::Fhir(Box::new(target)), value))
            })
            .collect();
        let openehr = entry
            .openehr
            .iter()
            .enumerate()
            .filter_map(|(index, manual)| {
                let at = path.field("openehr").index(index);
                let value = self.manual_value(file, &manual.value, &at)?;
                let target = self.openehr_target(file.file(), owner, &manual.path, scope, &at)?;
                self.carried_tail(
                    file,
                    &format!("{}.{}", scope.prefix, entry.name.value()),
                    &target,
                    true,
                    manual.path.position(),
                    &at,
                );
                Some(ManualPath::new(Target::Openehr(Box::new(target)), value))
            })
            .collect();
        Manual::new(
            entry.name.value().clone(),
            fhir,
            openehr,
            entry.fhir_condition.as_ref().and_then(|condition| {
                self.condition(
                    file,
                    condition,
                    scope,
                    Direction::FhirToOpenehr,
                    &path.field("fhirCondition"),
                    scope.fhir.as_str(),
                )
            }),
            entry.openehr_condition.as_ref().and_then(|condition| {
                self.condition(
                    file,
                    condition,
                    scope,
                    Direction::OpenehrToFhir,
                    &path.field("openehrCondition"),
                    &scope.openehr.to_string(),
                )
            }),
            entry.value.as_ref().map(|value| value.value().clone()),
            entry.unidirectional.as_ref().map(|value| *value.value()),
        )
    }

    /// Reads what one `manual` path writes: a literal or a `$context` member.
    ///
    /// "`$context` holds values passed in on the REST call ... a context value
    /// is referenced from a `manual` `value`"
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`,
    /// §`$context`), and every example names a member of it, so a bare
    /// `$context` names no value and is refused.
    fn manual_value(
        &mut self,
        file: &'a ModelMappingFile,
        written: &Located<String>,
        path: &ModelPath,
    ) -> Option<ManualValue> {
        let text = written.value();
        let Some(rest) = text.strip_prefix("$context") else {
            return Some(ManualValue::Literal(text.clone()));
        };
        let Some(member) = rest.strip_prefix('.').filter(|name| !name.is_empty()) else {
            self.diagnostics.push(diagnostic(
                file.file(),
                file.header().name().value(),
                ResolveCode::MalformedContextValue,
                written.position(),
                &path.field("value"),
                format!("`{text}` names no `$context` member, so it carries no value"),
            ));
            return None;
        };
        Some(ManualValue::Context(member.to_owned()))
    }

    /// Compiles one condition, on the side the direction names.
    fn condition(
        &mut self,
        file: &'a ModelMappingFile,
        condition: &Condition,
        scope: &Scope,
        direction: Direction,
        path: &ModelPath,
        guard: &str,
    ) -> Option<CompiledCondition> {
        let owner = file.header().name().value();
        let on_fhir = direction == Direction::FhirToOpenehr;
        let root = if on_fhir {
            self.fhir_target(
                file.file(),
                owner,
                &condition.target_root,
                scope,
                &path.field("targetRoot"),
                Site::ReadOnly,
            )
            .map(|target| Target::Fhir(Box::new(target)))
        } else {
            self.openehr_target(
                file.file(),
                owner,
                &condition.target_root,
                scope,
                &path.field("targetRoot"),
            )
            .map(|target| Target::Openehr(Box::new(target)))
        }?;
        let inner = match root {
            Target::Fhir(ref target) => Scope {
                fhir: target.expression().clone(),
                ..scope.clone()
            },
            Target::Openehr(ref target) => Scope {
                openehr: target.path().clone(),
                ..scope.clone()
            },
        };
        let attributes = condition
            .target_attributes
            .iter()
            .enumerate()
            .filter_map(|(index, attribute)| {
                let at = path.field("targetAttributes").index(index);
                if on_fhir {
                    self.fhir_target(file.file(), owner, attribute, &inner, &at, Site::ReadOnly)
                        .map(|target| Target::Fhir(Box::new(target)))
                } else {
                    self.openehr_target(file.file(), owner, attribute, &inner, &at)
                        .map(|target| Target::Openehr(Box::new(target)))
                }
            })
            .collect();
        let attachment = attachment_of(&root, guard);
        Some(CompiledCondition::new(ConditionParts {
            direction,
            target: root,
            attributes,
            operator: *condition.operator.value(),
            criteria: condition
                .criteria
                .iter()
                .map(|criteria| criteria.value().clone())
                .collect(),
            identifying: condition
                .identifying
                .as_ref()
                .is_some_and(|value| *value.value()),
            attachment,
        }))
    }

    /// Compiles the FHIR side of a `hierarchy.with`, which is only read.
    fn fhir_side(
        &mut self,
        file: &'a ModelMappingFile,
        with: &With,
        scope: &Scope,
        path: &ModelPath,
    ) -> Option<FhirTarget> {
        self.fhir_side_at(file, with, scope, path, Site::ReadOnly)
    }

    /// Compiles the FHIR side of a `with`.
    fn fhir_side_at(
        &mut self,
        file: &'a ModelMappingFile,
        with: &With,
        scope: &Scope,
        path: &ModelPath,
        site: Site,
    ) -> Option<FhirTarget> {
        let written = with.fhir.as_ref()?;
        self.fhir_target(
            file.file(),
            file.header().name().value(),
            written,
            scope,
            &path.field("fhir"),
            site,
        )
    }

    /// Compiles the openEHR side of a `with`.
    fn openehr_side(
        &mut self,
        file: &'a ModelMappingFile,
        with: &With,
        scope: &Scope,
        path: &ModelPath,
    ) -> Option<OpenehrTarget> {
        let written = with.openehr.as_ref()?;
        self.openehr_target(
            file.file(),
            file.header().name().value(),
            written,
            scope,
            &path.field("openehr"),
        )
    }

    /// Binds one FHIR expression to its anchor and resolves it against the
    /// element table.
    ///
    /// A mapping runs both ways unless `unidirectional` pins it to one
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`,
    /// §Direction), so an expression that cannot be written through is refused
    /// at a [`Site::Write`] unless the mapping only ever reads FHIR. A
    /// [`Site::ReadOnly`] expression is never written, so it takes every step
    /// the grammar defines.
    fn fhir_target(
        &mut self,
        file: &Path,
        owner: &MappingName,
        written: &Located<String>,
        scope: &Scope,
        path: &ModelPath,
        site: Site,
    ) -> Option<FhirTarget> {
        let expression = match FhirPath::from_str(written.value()) {
            Ok(expression) => expression,
            Err(error) => {
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    ResolveCode::MalformedFhirPath,
                    written.position(),
                    path,
                    format!(
                        "`{}` is not a FHIR path expression: {error}",
                        written.value()
                    ),
                ));
                return None;
            }
        };
        let mut anchors: Vec<&FhirPath> = vec![&scope.fhir];
        anchors.extend(scope.enclosing.iter().map(|outer| &outer.fhir));
        let (anchored, bound_in) = match expression.anchored_in(&anchors) {
            Ok(anchored) => anchored,
            Err(error) => {
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    ResolveCode::UnanchoredFhirPath,
                    written.position(),
                    path,
                    error.to_string(),
                ));
                return None;
            }
        };
        let resource = bound_in
            .checked_sub(1)
            .and_then(|outer| scope.enclosing.get(outer))
            .map_or(&scope.resource, |outer| &outer.resource);
        if let Writability::ReadOnly { ref step, reason } = *anchored.writability()
            && let Site::Write(direction) = site
            && direction != Some(Direction::FhirToOpenehr)
        {
            self.diagnostics.push(diagnostic(
                file,
                owner,
                ResolveCode::ReadOnlyFhirWrite,
                written.position(),
                path,
                format!(
                    "`{anchored}` cannot be written through, because `{step}` {reason}, and the \
                     mapping is not pinned to `fhir->openehr`"
                ),
            ));
            return None;
        }
        match resolve_element(self.table, resource.as_str(), &anchored) {
            Ok(resolved) => Some(FhirTarget::new(anchored, resolved)),
            Err(error) => {
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    ResolveCode::UnknownFhirElement,
                    written.position(),
                    path,
                    error.to_string(),
                ));
                None
            }
        }
    }

    /// Binds one openEHR path to its anchor and resolves it against the Web
    /// Template.
    fn openehr_target(
        &mut self,
        file: &Path,
        owner: &MappingName,
        written: &Located<String>,
        scope: &Scope,
        path: &ModelPath,
    ) -> Option<OpenehrTarget> {
        let parsed = match MappingPath::from_str(written.value()) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    ResolveCode::MalformedOpenehrPath,
                    written.position(),
                    path,
                    error.to_string(),
                ));
                return None;
            }
        };
        let anchor = self.openehr_anchor(file, owner, written, scope, path, &parsed)?;
        let resolved = match parsed.resolve(&anchor) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.diagnostics
                    .push(error.to_diagnostic(file.to_path_buf()));
                return None;
            }
        };
        if let Some(segment) = resolved
            .segments
            .iter()
            .find(|segment| segment.predicate.position.is_some())
        {
            self.diagnostics.push(diagnostic(
                file,
                owner,
                ResolveCode::MalformedOpenehrPath,
                written.position(),
                path,
                format!(
                    "`{resolved}` selects the instance `{}` inside a path, and occurrences are \
                     structured",
                    segment.attribute
                ),
            ));
            return None;
        }
        match self.locate(&resolved) {
            Ok((node, tail, leaf)) => {
                let occurrences = match occurrences(self.template, node) {
                    Ok(axes) => axes,
                    Err(error) => {
                        self.diagnostics.push(diagnostic(
                            file,
                            owner,
                            ResolveCode::UnknownTemplateNode,
                            written.position(),
                            path,
                            format!(
                                "the occurrence axes of `{}` do not resolve: {error}",
                                node.flat_id().as_str()
                            ),
                        ));
                        return None;
                    }
                };
                let target = OpenehrTarget::new(resolved, node.clone(), tail, occurrences);
                Some(match leaf {
                    Some(class) => target.with_leaf_class(class),
                    None => target,
                })
            }
            Err(error) => {
                let (code, message) = match error {
                    LocateError::Unknown(message) => (ResolveCode::UnknownTemplateNode, message),
                    LocateError::Ambiguous(message) => {
                        (ResolveCode::AmbiguousTemplateNode, message)
                    }
                };
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    code,
                    written.position(),
                    path,
                    message,
                ));
                None
            }
        }
    }

    /// Returns the path the variable an openEHR path opens with names.
    ///
    /// The five openEHR-side variables are `$archetype`, `$openehrRoot`,
    /// `$composition` and `$reference`, plus the anchor a path with no
    /// variable takes
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`).
    fn openehr_anchor(
        &mut self,
        file: &Path,
        owner: &MappingName,
        written: &Located<String>,
        scope: &Scope,
        path: &ModelPath,
        parsed: &MappingPath,
    ) -> Option<RmPath> {
        let Some(name) = parsed
            .variable()
            .map(openehr_mapping_core::path::PathVariable::name)
        else {
            return Some(scope.openehr.clone());
        };
        match Variable::from_str(name) {
            Ok(Variable::Archetype) => Some(scope.archetype.clone()),
            Ok(Variable::OpenehrRoot) => Some(scope.openehr.clone()),
            Ok(Variable::Composition) => Some(RmPath {
                absolute: true,
                segments: Vec::new(),
            }),
            // NOTE: `$reference` "indicates that there is no direct mapping
            // to openEHR" (`basics/Variables.adoc`), so the mapping
            // legitimately has no openEHR side rather than a broken one.
            Ok(Variable::Reference) => None,
            Ok(Variable::Resource | Variable::FhirRoot | Variable::Context) | Err(_) => {
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    ResolveCode::UnboundPathVariable,
                    written.position(),
                    path,
                    format!(
                        "`{}` opens an openEHR path with `${name}`, which names no openEHR anchor",
                        written.value()
                    ),
                ));
                None
            }
        }
    }

    /// Returns the deepest template node an openEHR path names, and the
    /// reference-model attributes below it.
    ///
    /// A Web Template carries the archetype roots and the leaves and compacts
    /// what lies between, so three forms resolve: a path that names an indexed
    /// node, a path that names an `ELEMENT` the builder compacted into its
    /// value node, and a path that reaches past the deepest indexed node into
    /// the reference model or through a structure the template compacted away.
    /// A segment the template carries nowhere and that constrains a node
    /// identity is refused. No specification governs this: our own design.
    fn locate(
        &self,
        path: &RmPath,
    ) -> Result<(&'a ResolvedNode, RmPath, Option<String>), LocateError> {
        let total = path.segments.len();
        for taken in (0..=total).rev() {
            let head = path.segments.get(..taken).unwrap_or_default();
            let tail = path.segments.get(taken..).unwrap_or_default();
            let prefix = RmPath {
                absolute: path.absolute,
                segments: head.to_vec(),
            };
            let rest = RmPath {
                absolute: false,
                segments: tail.to_vec(),
            };
            let node = match self.template.at_rm_path(&prefix) {
                Ok(node) => node,
                Err(PathError::AmbiguousPath { .. }) => {
                    return Err(self.ambiguity(&prefix, &self.template.all_at_rm_path(&prefix)));
                }
                Err(_) => {
                    let found: Vec<&ResolvedNode> = self
                        .compacted
                        .iter()
                        .filter(|entry| names_same_place(&prefix, &entry.0))
                        .map(|&(_, node)| node)
                        .collect();
                    match *found.as_slice() {
                        [only] => only,
                        [] => continue,
                        _ => return Err(self.ambiguity(&prefix, &found)),
                    }
                }
            };
            let constrained = rest.segments.iter().find(|segment| constrains(segment));
            if let Some(segment) = constrained
                && !self
                    .interior
                    .iter()
                    .any(|interior| names_same_place(path, interior))
            {
                return Err(LocateError::Unknown(format!(
                    "the template `{}` has no node at `{path}`; the deepest node it reaches is \
                     `{}` and `{}` below it names a node identity the template does not carry",
                    self.template.template_id(),
                    node.aql_path().as_str(),
                    segment.attribute
                )));
            }
            let leaf = rm_tail(node.rm_type(), &rest).map_err(|message| {
                LocateError::Unknown(format!(
                    "the template `{}` reaches `{}` and `{path}` walks `{rest}` below it: \
                     {message}",
                    self.template.template_id(),
                    node.aql_path().as_str()
                ))
            })?;
            return Ok((node, rest, leaf));
        }
        Err(LocateError::Unknown(format!(
            "the template `{}` has no node at `{path}`",
            self.template.template_id()
        )))
    }

    /// Builds the refusal for a path that names more than one template node.
    fn ambiguity(&self, path: &RmPath, candidates: &[&ResolvedNode]) -> LocateError {
        let named: Vec<&str> = candidates
            .iter()
            .map(|node| node.flat_id().as_str())
            .collect();
        LocateError::Ambiguous(format!(
            "`{path}` names {} nodes of the template `{}`: {}",
            named.len(),
            self.template.template_id(),
            named.join(", ")
        ))
    }
}

/// Why an openEHR path names no single node of the operational template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LocateError {
    /// No node of the template carries the path.
    Unknown(String),
    /// More than one node carries it, so nothing says which one is meant.
    Ambiguous(String),
}

/// Returns whether a method writes one of the mapping methods that do more
/// than map its two paths, whether or not the method compiled.
fn declares_method(method: &crate::model::ast::Mapping) -> bool {
    method.reference.is_some()
        || method.slot_archetype.is_some()
        || method.link.is_some()
        || method.mapping_code.is_some()
        || method.participations_function.is_some()
}

/// Returns the `spec` keys a method inherits from the file that wrote it.
///
/// An extension file is a file
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`),
/// so its `spec` governs the methods it contributes, wherever they land in the
/// merged model mapping.
fn inherited_spec(file: &ModelMappingFile, scope: &Scope) -> (Option<Direction>, Option<String>) {
    if file.file() == scope.file {
        return (scope.direction, scope.conceptmap.clone());
    }
    let direction = file
        .spec()
        .unidirectional
        .as_ref()
        .map_or(scope.direction, |located| Some(*located.value()));
    let conceptmap = file
        .spec()
        .conceptmap
        .as_ref()
        .map(|url| url.value().clone())
        .or_else(|| scope.conceptmap.clone());
    (direction, conceptmap)
}

/// Decides how a condition's `targetRoot` stands to the path it guards.
///
/// Both paths are anchored by the time this runs, so the comparison is over
/// the resolved paths rather than the text a file wrote.
fn attachment_of(root: &Target, guard: &str) -> Attachment {
    let written = match *root {
        Target::Fhir(ref target) => target.expression().as_str().to_owned(),
        Target::Openehr(ref target) => target.path().to_string(),
    };
    if written == guard {
        return Attachment::Element;
    }
    if walks_below(&written, guard) {
        return Attachment::Descendant;
    }
    if walks_below(guard, &written) {
        return Attachment::Ancestor;
    }
    Attachment::Unrelated
}

/// Whether `candidate` walks below `ancestor` in either path syntax.
fn walks_below(candidate: &str, ancestor: &str) -> bool {
    candidate
        .strip_prefix(ancestor)
        .is_some_and(|tail| tail.starts_with(['.', '/', '[']))
}

/// Returns the `$resource` expression every FHIR path is rooted at.
fn root_expression() -> FhirPath {
    RESOURCE_ROOT.clone()
}

/// The `$resource` expression, parsed once.
static RESOURCE_ROOT: LazyLock<FhirPath> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "`$resource` is a head the expression grammar defines, and \
                  the_resource_head_parses pins that it does"
    )]
    let root = FhirPath::from_str("$resource").expect("`$resource` should parse as an expression");
    root
});

/// Checks the reference-model attributes an openEHR path walks below the
/// deepest template node it reaches.
///
/// A Web Template carries the nodes an archetype constrains and stops there, so
/// everything below the deepest one is plain reference model. The attribute
/// model `openehr-rm` generates from the RM BMM is the oracle for it
/// (<https://docs.rs/openehr-rm/0.0.69/openehr_rm/v1_2/model/fn.attribute.html>):
/// an attribute is looked up on the node's type and on the concrete subtypes of
/// it, because a declared type may be abstract (`ELEMENT.value` is
/// `DATA_VALUE`) and the attribute then belongs to one of its descendants.
///
/// A type the model does not carry stops the walk rather than refusing it: the
/// mapping is then checked as far as the model reaches and no further.
///
/// Returns the class of the last attribute the tail names, `None` for an
/// empty tail and for a walk the model stopped before its end. The class is
/// the declared type, or its one concrete descendant when the declared type is
/// abstract and has exactly one.
fn rm_tail(rm_type: &str, tail: &RmPath) -> Result<Option<String>, String> {
    let mut candidates = concrete_forms(rm_type);
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut leaf: Option<&'static str> = None;
    for (position, segment) in tail.segments.iter().enumerate() {
        let attribute = segment.attribute.as_str();
        let mut declared: Option<&'static str> = None;
        let mut next: Vec<&'static str> = Vec::new();
        for candidate in &candidates {
            let Some(found) = rm_model::attribute(candidate, attribute) else {
                continue;
            };
            declared.get_or_insert(found.declared_type);
            for form in concrete_forms(found.declared_type) {
                if !next.contains(&form) {
                    next.push(form);
                }
            }
        }
        let Some(declared) = declared else {
            return Err(format!(
                "`{attribute}` is no attribute of `{}` in the openEHR reference model",
                candidates.join("`, `")
            ));
        };
        leaf = Some(declared);
        if next.is_empty() {
            let last = position.saturating_add(1) == tail.segments.len();
            return Ok(last.then(|| String::from(declared)));
        }
        candidates = next;
    }
    Ok(leaf.map(concrete_leaf))
}

/// Returns the one concrete form of an abstract class, or the class itself.
fn concrete_leaf(declared: &'static str) -> String {
    let Some(class) = rm_model::class(declared) else {
        return String::from(declared);
    };
    if !class.is_abstract {
        return String::from(declared);
    }
    let concrete: Vec<&str> = class
        .descendants
        .iter()
        .copied()
        .filter(|name| rm_model::class(name).is_some_and(|found| !found.is_abstract))
        .collect();
    match *concrete.as_slice() {
        [only] => String::from(only),
        _ => String::from(declared),
    }
}

/// Returns the reference-model class plus every concrete class below it.
///
/// An empty result means the reference model carries no class of that name,
/// which is what stops [`rm_tail`] rather than refusing.
fn concrete_forms(rm_type: &str) -> Vec<&'static str> {
    let Some(class) = rm_model::class(rm_type) else {
        return Vec::new();
    };
    let mut forms: Vec<&'static str> = vec![class.name];
    for descendant in class.descendants {
        if !forms.contains(descendant) {
            forms.push(descendant);
        }
    }
    forms
}

/// Whether a path segment constrains which node it selects.
fn constrains(segment: &PathSegment) -> bool {
    segment.predicate.archetype_node_id.is_some()
        || segment.predicate.name_value.is_some()
        || segment.predicate.position.is_some()
}

/// Returns the repeating nodes from the root down to `node`, outermost first.
///
/// # Errors
///
/// Returns the refusal the index raised for a prefix that is not simply
/// absent, so a lost occurrence axis is a diagnostic rather than a node that
/// silently stops repeating.
fn occurrences(
    template: &WebTemplateIndex,
    node: &ResolvedNode,
) -> Result<Vec<openehr_mapping_core::index::FlatId>, PathError> {
    let mut axes = Vec::new();
    let mut prefix = String::new();
    for segment in node.flat_id().as_str().split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        let flat_id = openehr_mapping_core::index::FlatId::new(prefix.clone());
        let found = match template.node_by_flat_id(&flat_id) {
            Ok(found) => found,
            // NOTE: a flat id is built one level at a time and the index
            // carries a node only where the builder kept one, so an unknown
            // prefix is a level with no node of its own.
            Err(PathError::UnknownNode { .. }) => continue,
            Err(error) => return Err(error),
        };
        if found.repeats() {
            axes.push(flat_id);
        }
    }
    Ok(axes)
}

/// Returns the archetype a model mapping declares.
fn archetype_of(model: &ModelMappingFile) -> Option<&ArchetypeId> {
    model.header().archetype().map(Located::value)
}

/// Returns every path that lies on the way to a node of the template.
///
/// The builder compacts the structure between an archetype root and a leaf, so
/// a path that names one of those structures is a path the template knows
/// without carrying a node for it.
fn interior_paths(template: &WebTemplateIndex) -> Vec<RmPath> {
    let mut found: BTreeMap<String, RmPath> = BTreeMap::new();
    for node in template.nodes() {
        let path = node.rm_path();
        for taken in 1..path.segments.len() {
            let prefix = RmPath {
                absolute: path.absolute,
                segments: path.segments.get(..taken).unwrap_or_default().to_vec(),
            };
            found.entry(prefix.to_string()).or_insert(prefix);
        }
    }
    found.into_values().collect()
}

/// Maps the path a mapping writes for a compacted node onto the nodes it
/// reaches.
///
/// A shortened path that reaches more than one node keeps all of them, because
/// binding the mapping to one of them would bind it to a node nothing named.
fn compaction_map(template: &WebTemplateIndex) -> Vec<(RmPath, &ResolvedNode)> {
    let mut found: Vec<(RmPath, &ResolvedNode)> = Vec::new();
    for node in template.nodes() {
        let path = node.rm_path();
        let kept = path
            .segments
            .iter()
            .rposition(constrains)
            .map_or(0, |last| last.saturating_add(1));
        if kept == path.segments.len() {
            continue;
        }
        let shortened = RmPath {
            absolute: path.absolute,
            segments: path.segments.get(..kept).unwrap_or_default().to_vec(),
        };
        found.push((shortened, node));
    }
    found
}

/// Whether a path a mapping writes names the place `carried` holds in the
/// template.
///
/// A mapping names an archetype node by its node id, and the template may
/// constrain the same node further with a name, which the `aqlPath` carries.
/// A name the mapping does not write selects nothing, the rule the index
/// applies to an indexed node; a node id the mapping writes must be the one
/// the template carries, and one it does not write matches only a segment
/// that carries none. No specification governs this: our own design.
fn names_same_place(written: &RmPath, carried: &RmPath) -> bool {
    written.absolute == carried.absolute
        && written.segments.len() == carried.segments.len()
        && written
            .segments
            .iter()
            .zip(&carried.segments)
            .all(|(wanted, held)| {
                let node_id = match (
                    wanted.predicate.archetype_node_id.as_deref(),
                    held.predicate.archetype_node_id.as_deref(),
                ) {
                    (Some(wanted), Some(held)) => node_id_matches(wanted, held),
                    (None, None) => true,
                    (Some(_), None) | (None, Some(_)) => false,
                };
                let name = match (
                    wanted.predicate.name_value.as_deref(),
                    held.predicate.name_value.as_deref(),
                ) {
                    (Some(wanted), Some(held)) => wanted == held,
                    (Some(_) | None, None) | (None, Some(_)) => true,
                };
                !wanted.descendant
                    && wanted.attribute == held.attribute
                    && wanted.predicate.position == held.predicate.position
                    && node_id
                    && name
            })
}
