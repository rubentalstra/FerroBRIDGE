// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR side of a mapping, resolved against the element table.

use core::str::FromStr;
use std::path::Path;
use std::sync::LazyLock;

use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::position::Located;

use crate::model::ast::ModelMappingFile;
use crate::model::ast::With;
use crate::model::ast::keyword::Direction;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::target::FhirTarget;
use crate::tree::element::resolve as resolve_element;
use crate::tree::path::FhirPath;
use crate::tree::path::Writability;

use crate::resolve::compile::Compiler;
use crate::resolve::compile::Scope;
use crate::resolve::compile::Site;

impl<'a> Compiler<'a> {
    /// Compiles the FHIR side of a `hierarchy.with`, which is only read.
    pub(super) fn fhir_side(
        &mut self,
        file: &'a ModelMappingFile,
        with: &With,
        scope: &Scope,
        path: &ModelPath,
    ) -> Option<FhirTarget> {
        self.fhir_side_at(file, with, scope, path, Site::ReadOnly)
    }

    /// Compiles the FHIR side of a `with`.
    pub(super) fn fhir_side_at(
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

    /// Binds one FHIR expression to its anchor and resolves it against the
    /// element table.
    ///
    /// A mapping runs both ways unless `unidirectional` pins it to one
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`,
    /// §Direction), so an expression that cannot be written through is refused
    /// at a [`Site::Write`] unless the mapping only ever reads FHIR. A
    /// [`Site::ReadOnly`] expression is never written, so it takes every step
    /// the grammar defines.
    pub(super) fn fhir_target(
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
}

/// Returns the `$resource` expression every FHIR path is rooted at.
pub(super) fn root_expression() -> FhirPath {
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
