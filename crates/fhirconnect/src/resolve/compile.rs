// SPDX-FileCopyrightText: Ruben Talstra
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
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::LazyLock;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::ArchetypeId;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::header::MappingType;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::index::archetype_release_version;
use openehr_mapping_core::index::node_id_matches;
use openehr_mapping_core::path::MappingPath;
use openehr_mapping_core::position::Located;
use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::RmPath;

use crate::model::ast::Condition;
use crate::model::ast::ContextMappingFile;
use crate::model::ast::Direction;
use crate::model::ast::ManualEntry;
use crate::model::ast::ModelMappingFile;
use crate::model::ast::SplitTarget;
use crate::model::ast::Variable;
use crate::model::ast::With;
use crate::model::load::MappingSet;
use crate::model::semantic::MappingCodeRegistry;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::Merged;
use crate::resolve::extensions::Node;
use crate::resolve::extensions::apply;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::Condition as CompiledCondition;
use crate::resolve::program::FhirTarget;
use crate::resolve::program::Hierarchy;
use crate::resolve::program::Manual;
use crate::resolve::program::Mapping;
use crate::resolve::program::MappingParts;
use crate::resolve::program::Method;
use crate::resolve::program::ModelBinding;
use crate::resolve::program::OpenehrTarget;
use crate::resolve::program::Pin;
use crate::resolve::program::ProfileBinding;
use crate::resolve::program::ProfileUrl;
use crate::resolve::program::Program;
use crate::resolve::program::ProgramParts;
use crate::resolve::program::ResourceType;
use crate::resolve::program::Split;
use crate::resolve::program::Target;
use crate::resolve::program::TemplateBinding;
use crate::resolve::program::TemplateId;
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
    match program {
        Some(program) if compiler.diagnostics.is_empty() => Ok(Arc::new(program)),
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

/// The compiler's state for one run.
struct Compiler<'a> {
    set: &'a MappingSet,
    template: &'a WebTemplateIndex,
    table: &'a dyn Table,
    codes: &'a dyn MappingCodeRegistry,
    compacted: BTreeMap<String, &'a ResolvedNode>,
    interior: BTreeSet<String>,
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
        let file = self.set.context(context)?;
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
        let archetype = self.archetype_root(start, file.file(), context)?;
        let scope = Scope {
            resource: resource.clone(),
            fhir: root_expression(),
            archetype: archetype.clone(),
            openehr: archetype,
            direction: start.spec().unidirectional.as_ref().map(|d| *d.value()),
            prefix: String::new(),
            chain: vec![start_name.clone()],
        };
        let merged = self.merge(start);
        let mappings = self.mappings(&merged.mappings, &scope);
        let (fhir_condition, openehr_condition, hierarchy) = self.preprocessor(start, &scope);
        Some(Program::new(ProgramParts {
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
            hierarchy,
            fhir_condition,
            openehr_condition,
            mappings,
        }))
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

    /// Applies the extensions that extend `model`, in declaration order.
    fn merge(&mut self, model: &'a ModelMappingFile) -> Merged<'a> {
        let name = model.header().name().value();
        let mine: Vec<&'a ModelMappingFile> = self
            .extensions
            .iter()
            .copied()
            .filter(|extension| {
                extension
                    .spec()
                    .extends
                    .as_ref()
                    .is_some_and(|extends| extends.value() == name)
            })
            .collect();
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
    /// (<https://hl7.org/fhir/R4/structuredefinition.html>).
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
    fn archetype_root(
        &mut self,
        model: &ModelMappingFile,
        file: &Path,
        owner: &MappingName,
    ) -> Option<RmPath> {
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
            let root = RmPath::from_str(only.aql_path().as_str()).ok();
            self.check_revision(model, file, owner, only.node_id());
            return root;
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
    fn check_revision(
        &mut self,
        model: &ModelMappingFile,
        file: &Path,
        owner: &MappingName,
        carried: Option<&str>,
    ) {
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
            file,
            owner,
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

    /// Compiles the preprocessor of the start model mapping.
    fn preprocessor(
        &mut self,
        model: &'a ModelMappingFile,
        scope: &Scope,
    ) -> (
        Option<CompiledCondition>,
        Option<CompiledCondition>,
        Option<Hierarchy>,
    ) {
        let Some(preprocessor) = model.preprocessor() else {
            return (None, None, None);
        };
        let path = ModelPath::root().field("preprocessor");
        let fhir = preprocessor.fhir_condition.as_ref().and_then(|condition| {
            self.condition(
                model,
                condition,
                scope,
                Direction::FhirToOpenehr,
                &path.field("fhirCondition"),
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
        (fhir, openehr, hierarchy)
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
                self.fhir_target(model.file(), owner, written, scope, &at, None)
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
                    self.fhir_target(model.file(), owner, written, scope, &at, None)
                        .map(|target| Target::Fhir(Box::new(target)))
                }
            })
            .collect();
        Split::new(
            target.create.as_ref().map(|create| create.value().clone()),
            created,
            unique,
        )
    }

    /// Compiles a list of mapping methods under one scope.
    fn mappings(&mut self, nodes: &[Node<'a>], scope: &Scope) -> Vec<Mapping> {
        nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let path = ModelPath::root().field("mappings").index(index);
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
        let direction = method
            .unidirectional
            .as_ref()
            .map_or(scope.direction, |located| Some(*located.value()));
        let with = method.with.as_ref();
        let fhir = with.and_then(|with| {
            self.fhir_side_with_direction(file, with, scope, &path.field("with"), direction)
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
            prefix: name.clone(),
            ..scope.clone()
        };
        let fhir_condition = method.fhir_condition.as_ref().and_then(|condition| {
            self.condition(
                file,
                condition,
                scope,
                Direction::FhirToOpenehr,
                &path.field("fhirCondition"),
            )
        });
        let openehr_condition = method.openehr_condition.as_ref().and_then(|condition| {
            self.condition(
                file,
                condition,
                scope,
                Direction::OpenehrToFhir,
                &path.field("openehrCondition"),
            )
        });
        let manual = method
            .manual
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                self.manual(file, entry, &inner, &path.field("manual").index(index))
            })
            .collect();
        let method_kind = self.method(node, &inner, path);
        let followed_by = self.mappings(&node.followed_by, &inner);
        Mapping::new(MappingParts {
            name,
            model: owner.clone(),
            fhir,
            openehr,
            data_type: with.and_then(|with| with.data_type.as_ref().map(|kind| *kind.value())),
            value: with.and_then(|with| with.value.as_ref().map(|value| value.value().clone())),
            direction,
            fhir_condition,
            openehr_condition,
            manual,
            conceptmap: method
                .conceptmap
                .as_ref()
                .map(|conceptmap| conceptmap.value().clone()),
            method: method_kind,
            followed_by,
        })
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
            let inner = Scope {
                resource: resource.clone(),
                fhir: root_expression(),
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
        if let (Some(archetype), Ok((node, _))) = (archetype, self.locate(&scope.openehr)) {
            match node.node_id() {
                Some(carried) if node_id_matches(archetype.as_str(), carried) => {
                    self.check_revision(slotted, file.file(), owner, Some(carried));
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
            prefix: String::new(),
            chain,
            ..scope.clone()
        };
        let merged = self.merge(slotted);
        Method::Slot {
            model: slot.value().clone(),
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
        let fhir = entry
            .fhir
            .iter()
            .enumerate()
            .filter_map(|(index, manual)| {
                let at = path.field("fhir").index(index);
                self.fhir_target(file.file(), owner, &manual.path, scope, &at, None)
                    .map(|target| {
                        crate::resolve::program::ManualPath::new(
                            Target::Fhir(Box::new(target)),
                            manual.value.value().clone(),
                        )
                    })
            })
            .collect();
        let openehr = entry
            .openehr
            .iter()
            .enumerate()
            .filter_map(|(index, manual)| {
                let at = path.field("openehr").index(index);
                self.openehr_target(file.file(), owner, &manual.path, scope, &at)
                    .map(|target| {
                        crate::resolve::program::ManualPath::new(
                            Target::Openehr(Box::new(target)),
                            manual.value.value().clone(),
                        )
                    })
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
                )
            }),
            entry.openehr_condition.as_ref().and_then(|condition| {
                self.condition(
                    file,
                    condition,
                    scope,
                    Direction::OpenehrToFhir,
                    &path.field("openehrCondition"),
                )
            }),
            entry.value.as_ref().map(|value| value.value().clone()),
            entry.unidirectional.as_ref().map(|value| *value.value()),
        )
    }

    /// Compiles one condition, on the side the direction names.
    fn condition(
        &mut self,
        file: &'a ModelMappingFile,
        condition: &Condition,
        scope: &Scope,
        direction: Direction,
        path: &ModelPath,
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
                None,
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
                    self.fhir_target(file.file(), owner, attribute, &inner, &at, None)
                        .map(|target| Target::Fhir(Box::new(target)))
                } else {
                    self.openehr_target(file.file(), owner, attribute, &inner, &at)
                        .map(|target| Target::Openehr(Box::new(target)))
                }
            })
            .collect();
        Some(CompiledCondition::new(
            direction,
            root,
            attributes,
            *condition.operator.value(),
            condition
                .criteria
                .iter()
                .map(|criteria| criteria.value().clone())
                .collect(),
            condition
                .identifying
                .as_ref()
                .is_some_and(|value| *value.value()),
        ))
    }

    /// Compiles the FHIR side of a `with`, with no direction check.
    fn fhir_side(
        &mut self,
        file: &'a ModelMappingFile,
        with: &With,
        scope: &Scope,
        path: &ModelPath,
    ) -> Option<FhirTarget> {
        self.fhir_side_with_direction(file, with, scope, path, None)
    }

    /// Compiles the FHIR side of a `with`.
    fn fhir_side_with_direction(
        &mut self,
        file: &'a ModelMappingFile,
        with: &With,
        scope: &Scope,
        path: &ModelPath,
        direction: Option<Direction>,
    ) -> Option<FhirTarget> {
        let written = with.fhir.as_ref()?;
        self.fhir_target(
            file.file(),
            file.header().name().value(),
            written,
            scope,
            &path.field("fhir"),
            direction,
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
    /// here unless the mapping only ever reads FHIR.
    fn fhir_target(
        &mut self,
        file: &Path,
        owner: &MappingName,
        written: &Located<String>,
        scope: &Scope,
        path: &ModelPath,
        direction: Option<Direction>,
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
        let anchored = match expression.anchored(&scope.fhir) {
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
        if let Writability::ReadOnly { ref step, reason } = *anchored.writability()
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
        match resolve_element(self.table, scope.resource.as_str(), &anchored) {
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
        let anchor = match parsed
            .variable()
            .map(openehr_mapping_core::path::PathVariable::name)
        {
            None => scope.openehr.clone(),
            Some(name) => match Variable::from_str(name) {
                Ok(Variable::Archetype) => scope.archetype.clone(),
                Ok(Variable::OpenehrRoot) => scope.openehr.clone(),
                Ok(Variable::Composition) => RmPath {
                    absolute: true,
                    segments: Vec::new(),
                },
                Ok(Variable::Reference) => return None,
                Ok(Variable::Resource | Variable::FhirRoot | Variable::Context) | Err(_) => {
                    self.diagnostics.push(diagnostic(
                        file,
                        owner,
                        ResolveCode::UnboundPathVariable,
                        written.position(),
                        path,
                        format!(
                            "`{}` opens an openEHR path with `${name}`, which names no openEHR \
                             anchor",
                            written.value()
                        ),
                    ));
                    return None;
                }
            },
        };
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
            Ok((node, tail)) => Some(OpenehrTarget::new(
                resolved,
                node.clone(),
                tail,
                occurrences(self.template, node),
            )),
            Err(message) => {
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    ResolveCode::UnknownTemplateNode,
                    written.position(),
                    path,
                    message,
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
    fn locate(&self, path: &RmPath) -> Result<(&'a ResolvedNode, RmPath), String> {
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
            let found = self
                .template
                .at_rm_path(&prefix)
                .ok()
                .or_else(|| self.compacted.get(&prefix.to_string()).copied());
            let Some(node) = found else {
                continue;
            };
            let constrained = rest.segments.iter().find(|segment| constrains(segment));
            if let Some(segment) = constrained
                && !self.interior.contains(&path.to_string())
            {
                return Err(format!(
                    "the template `{}` has no node at `{path}`; the deepest node it reaches is \
                     `{}` and `{}` below it names a node identity the template does not carry",
                    self.template.template_id(),
                    node.aql_path().as_str(),
                    segment.attribute
                ));
            }
            return Ok((node, rest));
        }
        Err(format!(
            "the template `{}` has no node at `{path}`",
            self.template.template_id()
        ))
    }
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

/// Whether a path segment constrains which node it selects.
fn constrains(segment: &PathSegment) -> bool {
    segment.predicate.archetype_node_id.is_some()
        || segment.predicate.name_value.is_some()
        || segment.predicate.position.is_some()
}

/// Returns the repeating nodes from the root down to `node`, outermost first.
fn occurrences(
    template: &WebTemplateIndex,
    node: &ResolvedNode,
) -> Vec<openehr_mapping_core::index::FlatId> {
    let mut axes = Vec::new();
    let mut prefix = String::new();
    for segment in node.flat_id().as_str().split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        let flat_id = openehr_mapping_core::index::FlatId::new(prefix.clone());
        if template
            .node_by_flat_id(&flat_id)
            .is_ok_and(ResolvedNode::repeats)
        {
            axes.push(flat_id);
        }
    }
    axes
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
fn interior_paths(template: &WebTemplateIndex) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for node in template.nodes() {
        let Ok(path) = RmPath::from_str(node.aql_path().as_str()) else {
            continue;
        };
        for taken in 1..path.segments.len() {
            let prefix = RmPath {
                absolute: path.absolute,
                segments: path.segments.get(..taken).unwrap_or_default().to_vec(),
            };
            found.insert(prefix.to_string());
        }
    }
    found
}

/// Maps the path a mapping writes for a compacted node onto that node.
fn compaction_map(template: &WebTemplateIndex) -> BTreeMap<String, &ResolvedNode> {
    let mut found: BTreeMap<String, Vec<&ResolvedNode>> = BTreeMap::new();
    for node in template.nodes() {
        let Ok(path) = RmPath::from_str(node.aql_path().as_str()) else {
            continue;
        };
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
        found.entry(shortened.to_string()).or_default().push(node);
    }
    found
        .into_iter()
        .filter_map(|(key, nodes)| match *nodes.as_slice() {
            [only] => Some((key, only)),
            _ => None,
        })
        .collect()
}
