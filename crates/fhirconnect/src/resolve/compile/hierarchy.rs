// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The preprocessors of a model mapping and their `hierarchy.split`.

use core::str::FromStr;

use openehr_mapping_core::diagnostic::ModelPath;

use crate::model::ast::ModelMappingFile;
use crate::model::ast::SplitTarget;
use crate::model::ast::keyword::Direction;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::hierarchy::Create;
use crate::resolve::program::hierarchy::Hierarchy;
use crate::resolve::program::hierarchy::Preprocessor;
use crate::resolve::program::hierarchy::Split;
use crate::resolve::program::target::Target;

use crate::resolve::compile::Compiler;
use crate::resolve::compile::Scope;
use crate::resolve::compile::Site;

impl<'a> Compiler<'a> {
    /// Compiles the preprocessor of `model` and of every extension of it.
    ///
    /// A preprocessor gates the file that wrote it
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
    /// §Conditions in the preprocessor), and an extension file is a file, so
    /// each contributes its own gate under the same anchors.
    pub(super) fn preprocessors(
        &mut self,
        model: &'a ModelMappingFile,
        scope: &Scope,
    ) -> Vec<Preprocessor> {
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
}
