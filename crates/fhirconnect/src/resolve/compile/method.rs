// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The concept methods, the slots and the `manual` entries of a mapping.

use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::matching::node_id_matches;
use openehr_mapping_core::position::Located;

use crate::model::ast::ManualEntry;
use crate::model::ast::ModelMappingFile;
use crate::model::ast::keyword::Direction;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::Node;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::binding::ResourceType;
use crate::resolve::program::manual::Manual;
use crate::resolve::program::manual::ManualPath;
use crate::resolve::program::manual::ManualValue;
use crate::resolve::program::mapping::Method;
use crate::resolve::program::target::Target;

use crate::resolve::compile::Compiler;
use crate::resolve::compile::Enclosing;
use crate::resolve::compile::Scope;
use crate::resolve::compile::Site;
use crate::resolve::compile::fhir::root_expression;
use crate::resolve::compile::header::archetype_of;

impl<'a> Compiler<'a> {
    /// Decides what a mapping method does beyond mapping its two paths.
    pub(super) fn method(&mut self, node: &Node<'a>, scope: &Scope, path: &ModelPath) -> Method {
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
    pub(super) fn manual(
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
}
