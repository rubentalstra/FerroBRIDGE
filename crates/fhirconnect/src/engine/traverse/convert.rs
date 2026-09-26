// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data-type cell of one mapping, run over its input values.

use fhir_types::codec::Value;
use openehr_mapping_core::composition::RmPosition;

use crate::engine::cell;
use crate::engine::fhir::FhirKind;
use crate::engine::fhir::FhirValue;
use crate::engine::lens::LensError;
use crate::engine::outcome::Warning;
use crate::model::ast::keyword::Direction;
use crate::resolve::program::manual::Manual;
use crate::resolve::program::manual::ManualValue;
use crate::resolve::program::mapping::Derived;
use crate::resolve::program::mapping::Mapping;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;
use crate::tree::Occurrence;

use crate::tree::element::Table;

use crate::engine::traverse::Binding;
use crate::engine::traverse::Leaf;
use crate::engine::traverse::Run;
use crate::engine::traverse::Written;
use crate::engine::traverse::error::EngineError;
use crate::engine::traverse::functions::MappingCodeError;
use crate::engine::traverse::place::rm_positions;

impl<T: Table + ?Sized> Run<'_, T> {
    /// Returns the text a `manual` path writes.
    ///
    /// A `$context` member is a per-call value the caller supplies, so a
    /// member this run does not carry is a refusal rather than an invented
    /// value (`basics/Variables.adoc`, §`$context`).
    pub(super) fn manual_text<'run>(
        &'run self,
        mapping: &Mapping,
        entry: &Manual,
        value: &'run ManualValue,
    ) -> Result<&'run str, EngineError> {
        match *value {
            ManualValue::Literal(ref text) => Ok(text.as_str()),
            ManualValue::Context(ref member) => {
                self.context
                    .member(member)
                    .ok_or_else(|| EngineError::ContextMember {
                        mapping: format!("{}.{}", mapping.name(), entry.name()),
                        member: member.clone(),
                    })
            }
        }
    }

    /// Runs the data-type cell of a plain `with` mapping, or only binds.
    pub(super) fn values(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        let bindings = self.bindings(mapping, parent, inputs)?;
        if !mapping.manual().is_empty() {
            for entry in mapping.manual() {
                for binding in &bindings {
                    self.manual(mapping, entry, binding)?;
                }
            }
            return Ok(bindings);
        }
        // NOTE: PopulatingAnEntry.adoc writes `type: NONE` on the parent whose
        // `followedBy` children carry every value, so the parent anchors them
        // and transforms nothing of its own (recorded as a silence in #185).
        if mapping.data_type() == Some(crate::model::ast::keyword::DataType::None)
            || matches!(mapping.derived(), Some(Derived::Anchor))
        {
            return Ok(bindings);
        }
        let (Some(_fhir), Some(_openehr)) = (mapping.fhir(), mapping.openehr()) else {
            return Ok(bindings);
        };
        for (input, binding) in inputs.iter().zip(bindings.iter()) {
            self.convert(mapping, input, binding)?;
        }
        Ok(bindings)
    }

    /// Runs the registered function a `mappingCode` names.
    pub(super) fn programmed(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        code: &str,
    ) -> Result<Vec<Binding>, EngineError> {
        let bindings = self.bindings(mapping, parent, inputs)?;
        let refuse = |source: MappingCodeError| EngineError::MappingCode {
            mapping: String::from(mapping.name()),
            code: String::from(code),
            source: Box::new(source),
        };
        for binding in &bindings {
            match self.direction {
                Direction::FhirToOpenehr => {
                    let produced = self
                        .seams
                        .functions()
                        .to_openehr(code, None)
                        .map_err(refuse)?;
                    self.put_openehr(mapping, binding, produced);
                }
                Direction::OpenehrToFhir => {
                    let produced = self.seams.functions().to_fhir(code, None).map_err(refuse)?;
                    self.put_fhir(mapping, binding, &produced)?;
                }
            }
        }
        Ok(bindings)
    }

    /// Runs one occurrence of a mapping through its data-type cell.
    fn convert(
        &mut self,
        mapping: &Mapping,
        input: &Occurrence,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        match self.direction {
            Direction::FhirToOpenehr => {
                let Some(target) = mapping.fhir() else {
                    return Ok(());
                };
                let Some(openehr) = mapping.openehr() else {
                    return Ok(());
                };
                let Some(view) = self.input_value(mapping, target, openehr, input)? else {
                    return Ok(());
                };
                let node = openehr.node();
                if !openehr.tail().segments.is_empty() {
                    return self.convert_tail(mapping, target, openehr, &view, binding);
                }
                let held = self.held(&binding.openehr, node.flat_id());
                let produced = cell::put(
                    &view,
                    node.rm_type(),
                    held.as_ref(),
                    self.binding_of(node).as_deref(),
                )
                .map_err(|source| EngineError::Cell {
                    mapping: String::from(mapping.name()),
                    element: String::from(target.expression().as_str()),
                    source: Box::new(source),
                })?;
                for taken in &produced.fallbacks {
                    self.warnings.push(Warning::fallback(taken));
                }
                self.put_openehr(mapping, binding, produced.value);
                Ok(())
            }
            Direction::OpenehrToFhir => {
                let Some(target) = mapping.fhir() else {
                    return Ok(());
                };
                let Some(openehr) = mapping.openehr() else {
                    return Ok(());
                };
                let positions = Self::positions_of(mapping, input, openehr)?;
                let Some(source) = self.value_at(mapping, openehr, &positions)? else {
                    return Ok(());
                };
                let (target, kind) = Self::output_target(mapping, target)?;
                let refuse = |source: LensError| EngineError::Cell {
                    mapping: String::from(mapping.name()),
                    element: String::from(target.expression().as_str()),
                    source: Box::new(source),
                };
                let view = match source {
                    Leaf::Value(ref value) => {
                        cell::get(value, kind, self.binding_of(openehr.node()).as_deref())
                            .map_err(refuse)?
                    }
                    // NOTE: a scalar leaf carries text, so only an element that
                    // travels as text takes it; no specification governs this:
                    // our own design.
                    Leaf::Scalar(ref text) => match kind {
                        FhirKind::String | FhirKind::DateTime => {
                            let element = target.resolved().leaf();
                            FhirValue::read(kind, element, &Value::String(text.clone())).map_err(
                                |source| EngineError::Fhir {
                                    mapping: String::from(mapping.name()),
                                    element: String::from(element),
                                    source: Box::new(source),
                                },
                            )?
                        }
                        FhirKind::Coding
                        | FhirKind::CodeableConcept
                        | FhirKind::Period
                        | FhirKind::Reference
                        | FhirKind::Identifier
                        | FhirKind::Quantity => {
                            return Err(refuse(LensError::NoCell {
                                rm_type: String::from(openehr.leaf_class().unwrap_or("String")),
                                kind: kind.as_str(),
                                direction: Direction::OpenehrToFhir,
                            }));
                        }
                    },
                };
                self.put_fhir_at(mapping, target, binding, &view)
            }
        }
    }

    /// Reads the FHIR element one input occurrence names, as the conversion
    /// the compiler derived or the `type` key names.
    fn input_value(
        &self,
        mapping: &Mapping,
        target: &FhirTarget,
        openehr: &OpenehrTarget,
        input: &Occurrence,
    ) -> Result<Option<FhirValue>, EngineError> {
        match mapping.derived() {
            Some(Derived::Choice { read, .. }) => self.choice_at(mapping, read, openehr, input),
            Some(Derived::Declared(declared)) => {
                let kind = Self::kind(mapping, declared.target())?;
                self.element_at(mapping, declared.target(), input, kind)
            }
            Some(Derived::Element(_) | Derived::Anchor) | None => {
                let kind = Self::kind(mapping, target)?;
                self.element_at(mapping, target, input, kind)
            }
        }
    }

    /// Returns the target a mapping writes FHIR through, with the element
    /// kind it writes.
    fn output_target<'mapping>(
        mapping: &'mapping Mapping,
        target: &'mapping FhirTarget,
    ) -> Result<(&'mapping FhirTarget, FhirKind), EngineError> {
        match mapping.derived() {
            Some(Derived::Choice {
                write: Some(written),
                ..
            }) => {
                let kind = FhirKind::of_code(written.code()).ok_or_else(|| {
                    EngineError::UnknownElement {
                        mapping: String::from(mapping.name()),
                        element: String::from(written.target().resolved().leaf()),
                    }
                })?;
                Ok((written.target(), kind))
            }
            Some(Derived::Declared(declared)) => {
                Ok((declared.target(), Self::kind(mapping, declared.target())?))
            }
            Some(Derived::Choice { write: None, .. }) => Err(EngineError::UnknownElement {
                mapping: String::from(mapping.name()),
                element: String::from(target.resolved().leaf()),
            }),
            Some(Derived::Element(_) | Derived::Anchor) | None => {
                Ok((target, Self::kind(mapping, target)?))
            }
        }
    }

    /// Stores one written value, overwriting what the same place holds.
    pub(super) fn store(&mut self, written: Written) {
        if let Some(slot) = self
            .written
            .iter_mut()
            .find(|held| held.flat_id == written.flat_id && held.positions == written.positions)
        {
            *slot = written;
        } else {
            self.written.push(written);
        }
    }

    /// Returns the positions one input occurrence names for an openEHR node.
    fn positions_of(
        mapping: &Mapping,
        input: &Occurrence,
        target: &OpenehrTarget,
    ) -> Result<Vec<RmPosition>, EngineError> {
        let depth = target.occurrences().len();
        let taken = input.indices().get(..depth).unwrap_or(input.indices());
        let mut positions = rm_positions(mapping.name(), taken)?;
        positions.resize(depth, RmPosition::first());
        Ok(positions)
    }
}
