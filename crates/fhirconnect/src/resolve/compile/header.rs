// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The headers of a context: its profile, its template, the extensions it
//! applies and the archetype a model mapping is rooted at.

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::archetype::ArchetypeId;
use openehr_mapping_core::header::metadata::MappingType;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::matching::archetype_release_version;
use openehr_mapping_core::index::matching::node_id_matches;
use openehr_mapping_core::position::Located;
use openehr_rm::v1_2::paths::RmPath;

use crate::model::ast::ContextMappingFile;
use crate::model::ast::ModelMappingFile;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::Merged;
use crate::resolve::extensions::apply;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::binding::ModelBinding;
use crate::resolve::program::binding::Pin;
use crate::resolve::program::binding::ProfileBinding;
use crate::resolve::program::binding::ProfileUrl;
use crate::resolve::program::binding::ResourceType;
use crate::resolve::program::binding::TemplateBinding;
use crate::resolve::program::binding::TemplateId;

use crate::resolve::compile::Compiler;

impl<'a> Compiler<'a> {
    /// Reports every listed extension whose target model the program never
    /// reached.
    ///
    /// An extension applies when the model mapping it extends is merged, so
    /// one whose model no slot of the chain reaches changes nothing. The
    /// context still compiles, and the listing is reported as a warning
    /// because an author who lists an extension expects it to act. No
    /// specification governs this: our own design.
    pub(super) fn unreached_extensions(&mut self, file: &ContextMappingFile) {
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
    pub(super) fn profile_of(file: &ContextMappingFile) -> ProfileBinding {
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
    pub(super) fn template_of(&mut self, file: &ContextMappingFile) -> TemplateBinding {
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
    pub(super) fn extensions_of(&mut self, file: &ContextMappingFile) -> Vec<&'a ModelMappingFile> {
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
    pub(super) fn extensions_for(&self, model: &'a ModelMappingFile) -> Vec<&'a ModelMappingFile> {
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
    pub(super) fn merge(&mut self, model: &'a ModelMappingFile) -> Merged<'a> {
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
    pub(super) fn resource_of(&mut self, model: &ModelMappingFile) -> Option<ResourceType> {
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
    pub(super) fn archetype_root(&mut self, model: &ModelMappingFile) -> Option<RmPath> {
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
    pub(super) fn check_revision(&mut self, model: &ModelMappingFile, carried: Option<&str>) {
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
}

/// Returns the archetype a model mapping declares.
pub(super) fn archetype_of(model: &ModelMappingFile) -> Option<&ArchetypeId> {
    model.header().archetype().map(Located::value)
}
