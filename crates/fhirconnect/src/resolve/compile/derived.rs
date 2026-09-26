// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pair a mapping with no `type` converts through.

use core::str::FromStr;

use fhir_types::schema::ValueKind;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::position::Position;

use crate::model::ast::ModelMappingFile;
use crate::model::ast::keyword::Direction;
use crate::resolve::derive;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::mapping::Alternative;
use crate::resolve::program::mapping::Derived;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;
use crate::tree::element::Location;
use crate::tree::element::Move;
use crate::tree::element::resolve as resolve_element;
use crate::tree::path::FhirPath;

use crate::resolve::compile::Compiler;
use crate::resolve::compile::Derivation;

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

impl<'a> Compiler<'a> {
    /// Derives what the data-type cell of a mapping converts, when the
    /// mapping leaves it to the compiler.
    ///
    /// A plain value mapping with no `type` key derives its pair from its two
    /// sides, and a mapping whose `type` key names the type of a choice element
    /// no filter resolved takes that alternative. Every other mapping carries
    /// what it wrote.
    pub(super) fn derived_of(
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
}
