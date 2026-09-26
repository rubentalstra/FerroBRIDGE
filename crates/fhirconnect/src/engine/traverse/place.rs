// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Where an output is written: the element a value lands on and the instance
//! the recurrence rule gives it.

use openehr_mapping_core::composition::FlatIndex;
use openehr_mapping_core::composition::PositionError;
use openehr_mapping_core::composition::RmPosition;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::paths::FlatId;

use crate::engine::fhir::FhirKind;
use crate::engine::fhir::FhirValue;
use crate::engine::outcome::Warning;
use crate::engine::recurrence::Cardinality;
use crate::engine::recurrence::Placement;
use crate::engine::rm::RmValue;
use crate::model::ast::keyword::Direction;
use crate::resolve::derive;
use crate::resolve::program::mapping::Derived;
use crate::resolve::program::mapping::Mapping;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;
use crate::tree::Occurrence;

use crate::tree::element::Move;
use crate::tree::element::Table;
use crate::tree::read::read;
use crate::tree::write::write;

use crate::engine::traverse::Binding;
use crate::engine::traverse::Held;
use crate::engine::traverse::Run;
use crate::engine::traverse::Written;
use crate::engine::traverse::error::EngineError;

impl<T: Table + ?Sized> Run<'_, T> {
    /// Returns which FHIR element a mapping writes.
    ///
    /// The `type` key names it where the mapping carries one, and the pair
    /// the compiler derived otherwise.
    pub(super) fn kind(mapping: &Mapping, target: &FhirTarget) -> Result<FhirKind, EngineError> {
        let kind = match (mapping.data_type(), mapping.derived()) {
            (Some(data_type), _) => FhirKind::of(data_type),
            (None, Some(Derived::Element(code))) => FhirKind::of_code(code),
            (None, _) => FhirKind::at(target.resolved().location()),
        };
        kind.ok_or_else(|| EngineError::UnknownElement {
            mapping: String::from(mapping.name()),
            element: String::from(target.resolved().leaf()),
        })
    }

    /// Reads the choice element one input occurrence names, as the
    /// alternative the document carries.
    ///
    /// The JSON key of a choice names its type
    /// (<https://hl7.org/fhir/R4/formats.html#choice>), so the instance fixes
    /// the kind, and the node's class decides how a text alternative reads
    /// (`crate::resolve::derive::carried`).
    pub(super) fn choice_at(
        &self,
        mapping: &Mapping,
        read_at: &FhirTarget,
        openehr: &OpenehrTarget,
        occurrence: &Occurrence,
    ) -> Result<Option<FhirValue>, EngineError> {
        let matches = read(self.table, &self.fhir, read_at.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(mapping.name()),
                expression: String::from(read_at.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        let Some(found) = matches
            .iter()
            .find(|matched| matched.occurrence() == occurrence)
        else {
            return Ok(None);
        };
        let (Some(value), Some((suffix, variant))) = (found.value(), found.alternative()) else {
            return Ok(None);
        };
        let element = read_at.resolved().leaf();
        let class = openehr.leaf_class().unwrap_or(openehr.node().rm_type());
        let kind = derive::carried(class, suffix, variant)
            .and_then(FhirKind::of_code)
            .ok_or_else(|| EngineError::UnknownElement {
                mapping: String::from(mapping.name()),
                element: format!("{element} as {suffix}"),
            })?;
        FhirValue::read(kind, element, value)
            .map(Some)
            .map_err(|source| EngineError::Fhir {
                mapping: String::from(mapping.name()),
                element: String::from(element),
                source: Box::new(source),
            })
    }

    /// Reads the FHIR element one input occurrence names.
    pub(super) fn element_at(
        &self,
        mapping: &Mapping,
        target: &FhirTarget,
        occurrence: &Occurrence,
        kind: FhirKind,
    ) -> Result<Option<FhirValue>, EngineError> {
        let matches = read(self.table, &self.fhir, target.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(mapping.name()),
                expression: String::from(target.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        let Some(found) = matches
            .iter()
            .find(|matched| matched.occurrence() == occurrence)
            .and_then(crate::tree::read::Match::value)
        else {
            return Ok(None);
        };
        FhirValue::read(kind, target.resolved().leaf(), found)
            .map(Some)
            .map_err(|source| EngineError::Fhir {
                mapping: String::from(mapping.name()),
                element: String::from(target.resolved().leaf()),
                source: Box::new(source),
            })
    }

    /// Returns the terminology the template binds an openEHR node to.
    pub(super) fn binding_of(&self, node: &ResolvedNode) -> Option<String> {
        self.index
            .bindings(node)
            .ok()
            .and_then(<[openehr_mapping_core::template::Binding]>::first)
            .map(|binding| String::from(binding.system()))
    }

    /// Returns the openEHR value the run already wrote at one place.
    ///
    /// A value still being written attribute by attribute is no whole value
    /// yet, so it holds nothing a cell can carry over.
    pub(super) fn held(&self, positions: &[RmPosition], flat_id: &FlatId) -> Option<RmValue> {
        self.written
            .iter()
            .find(|written| written.flat_id == *flat_id && written.positions == positions)
            .and_then(|written| match written.value {
                Held::Value(ref value) => Some(value.clone()),
                Held::Partial(_) => None,
            })
    }

    /// Writes one openEHR value, overwriting what the same place holds.
    pub(super) fn put_openehr(&mut self, mapping: &Mapping, binding: &Binding, value: RmValue) {
        let Some(target) = mapping.openehr() else {
            return;
        };
        self.store(Written {
            flat_id: target.node().flat_id().clone(),
            positions: binding.openehr.clone(),
            value: Held::Value(value),
        });
    }

    /// Writes one FHIR element at the occurrence its binding names.
    pub(super) fn put_fhir(
        &mut self,
        mapping: &Mapping,
        binding: &Binding,
        view: &FhirValue,
    ) -> Result<(), EngineError> {
        let Some(target) = mapping.fhir() else {
            return Ok(());
        };
        self.put_fhir_at(mapping, target, binding, view)
    }

    /// Writes one FHIR element through `target` at the occurrence its
    /// binding names.
    pub(super) fn put_fhir_at(
        &mut self,
        mapping: &Mapping,
        target: &FhirTarget,
        binding: &Binding,
        view: &FhirValue,
    ) -> Result<(), EngineError> {
        let element = String::from(target.resolved().leaf());
        let value = view.write(&element).map_err(|source| EngineError::Fhir {
            mapping: String::from(mapping.name()),
            element: element.clone(),
            source: Box::new(source),
        })?;
        write(
            self.table,
            &mut self.fhir,
            target.expression(),
            &binding.fhir,
            value,
        )
        .map_err(|source| EngineError::Write {
            mapping: String::from(mapping.name()),
            element,
            source: Box::new(source),
        })
    }

    /// Returns the next free instance of one axis under one prefix.
    pub(super) fn next_index(&mut self, axis: &str, prefix: &[usize]) -> usize {
        let key = (String::from(axis), render(prefix));
        let next = self.counters.entry(key).or_insert(0);
        let taken = *next;
        *next = next.saturating_add(1);
        taken
    }

    /// Returns the binding each input occurrence writes under.
    pub(super) fn bindings(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        let axes = self.output_axes(mapping);
        let mut bound = parent_depth(parent, self.direction);
        while axes
            .get(bound)
            .is_some_and(|axis| self.pin_named(axis).is_some())
        {
            bound = bound.saturating_add(1);
        }
        let cardinality = Cardinality::of(axes.len() > bound);
        let placement = Placement::decide(inputs.len(), cardinality, 0);
        if placement.lossy() {
            self.warnings.push(Warning::LastOfMany {
                path: self.output_name(mapping),
                dropped: placement.dropped(),
            });
        }
        let mut bindings = Vec::with_capacity(inputs.len());
        for (position, input) in inputs.iter().enumerate() {
            let taken = placement.slots().get(position).copied().flatten();
            bindings.push(self.bind(mapping, parent, input, &axes, taken)?);
        }
        Ok(bindings)
    }

    /// Returns one binding, appending at every axis the parent did not bind.
    fn bind(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        input: &Occurrence,
        axes: &[String],
        taken: Option<usize>,
    ) -> Result<Binding, EngineError> {
        let bound = parent_depth(parent, self.direction);
        let mut indices: Vec<usize> = match self.direction {
            // NOTE: an openEHR position is 1-based and an instance index is
            // 0-based (Simplified Formats §Instance Indexing), so the parent's
            // positions come back through the index the counters speak.
            Direction::FhirToOpenehr => {
                let mut held = Vec::with_capacity(parent.openehr.len());
                for position in &parent.openehr {
                    let index = FlatIndex::from(*position).get();
                    held.push(usize::try_from(index).map_err(|_refused| {
                        EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source: PositionError::Overflow,
                        }
                    })?);
                }
                held
            }
            Direction::OpenehrToFhir => parent.fhir.indices().to_vec(),
        };
        indices.truncate(bound);
        if taken.is_some() {
            let mut fresh = false;
            for axis in axes.iter().skip(bound) {
                let index = if let Some(pinned) = self.pin_named(axis) {
                    usize::try_from(FlatIndex::from(pinned).get()).map_err(|_refused| {
                        EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source: PositionError::Overflow,
                        }
                    })?
                } else if fresh {
                    0
                } else {
                    fresh = true;
                    let key = (axis.clone(), render(&indices));
                    let next = self.counters.entry(key).or_insert(0);
                    let taken = *next;
                    *next = next.saturating_add(1);
                    taken
                };
                indices.push(index);
            }
        }
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut positions = Vec::with_capacity(indices.len());
                for index in &indices {
                    let index =
                        u32::try_from(*index).map_err(|_refused| EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source: PositionError::Overflow,
                        })?;
                    positions.push(RmPosition::try_from(FlatIndex::new(index)).map_err(
                        |source| EngineError::Position {
                            mapping: String::from(mapping.name()),
                            source,
                        },
                    )?);
                }
                Ok(Binding {
                    fhir: input.clone(),
                    openehr: positions,
                })
            }
            Direction::OpenehrToFhir => Ok(Binding {
                fhir: Occurrence::new(indices),
                openehr: rm_positions(mapping.name(), input.indices())?,
            }),
        }
    }

    /// Returns the repeating elements of the output side, outermost first.
    fn output_axes(&self, mapping: &Mapping) -> Vec<String> {
        match self.direction {
            Direction::FhirToOpenehr => mapping.openehr().map_or_else(Vec::new, |target| {
                target
                    .occurrences()
                    .iter()
                    .map(|axis| String::from(axis.as_str()))
                    .collect()
            }),
            Direction::OpenehrToFhir => mapping.fhir().map_or_else(Vec::new, fhir_axes),
        }
    }

    /// Returns the name of the output side, for a declared loss.
    fn output_name(&self, mapping: &Mapping) -> String {
        match self.direction {
            Direction::FhirToOpenehr => mapping
                .openehr()
                .map(|target| String::from(target.node().aql_path().as_str()))
                .unwrap_or_default(),
            Direction::OpenehrToFhir => mapping
                .fhir()
                .map(|target| String::from(target.resolved().leaf()))
                .unwrap_or_default(),
        }
    }
}

/// Returns how many axes of the output side the parent bound.
fn parent_depth(parent: &Binding, direction: Direction) -> usize {
    match direction {
        Direction::FhirToOpenehr => parent.openehr.len(),
        Direction::OpenehrToFhir => parent.fhir.indices().len(),
    }
}

/// Returns the repeating elements a FHIR path steps through, outermost first.
///
/// Each axis is named by the JSON keys the walk took from the resource root,
/// so two elements that share a definition (`Condition.code.coding` and
/// `Condition.verificationStatus.coding` are both `CodeableConcept.coding`)
/// count their instances apart. An `extension(url)` step takes an index of
/// its own, because the url names one entry of a repeating element
/// (<https://hl7.org/fhir/R4/fhirpath.html>, §Additional functions).
pub(super) fn fhir_axes(target: &FhirTarget) -> Vec<String> {
    let mut walked = String::from(target.resolved().resource());
    let mut axes = Vec::new();
    for step in target.resolved().moves() {
        match *step {
            Move::Member(ref field) => {
                walked.push('.');
                walked.push_str(field.key());
                if field.repeats() {
                    axes.push(walked.clone());
                }
            }
            Move::Choice { ref field, .. } => {
                walked.push('.');
                walked.push_str(field.key());
            }
            Move::Extension { ref field, ref url } => {
                walked.push('.');
                walked.push_str(field.key());
                walked.push('(');
                walked.push_str(url);
                walked.push(')');
                axes.push(walked.clone());
            }
            Move::Ordinal(_) | Move::Index(_) | Move::Predicate(_) | Move::Resolve { .. } => {}
        }
    }
    axes
}

/// Returns the openEHR positions an openEHR input occurrence carries.
///
/// An openEHR occurrence holds 1-based positions (openEHR BASE Release 1.2.0
/// §Paths and Locators), so an index past `u32` is
/// [`PositionError::Overflow`] and a 0 is [`PositionError::Zero`], each a
/// refusal naming the mapping and never a dropped position.
pub(super) fn rm_positions(name: &str, indices: &[usize]) -> Result<Vec<RmPosition>, EngineError> {
    let refuse = |source: PositionError| EngineError::Position {
        mapping: String::from(name),
        source,
    };
    let mut positions = Vec::with_capacity(indices.len());
    for &index in indices {
        let index = u32::try_from(index).map_err(|_refused| refuse(PositionError::Overflow))?;
        positions.push(RmPosition::new(index).map_err(refuse)?);
    }
    Ok(positions)
}

/// Returns an openEHR input occurrence of the positions it names.
pub(super) fn openehr_occurrence(
    name: &str,
    positions: &[RmPosition],
) -> Result<Occurrence, EngineError> {
    let mut indices = Vec::with_capacity(positions.len());
    for position in positions {
        indices.push(usize::try_from(position.get()).map_err(|_refused| {
            EngineError::Position {
                mapping: String::from(name),
                source: PositionError::Overflow,
            }
        })?);
    }
    Ok(Occurrence::new(indices))
}

/// Renders an index prefix, for the counter that appends under it.
fn render(indices: &[usize]) -> String {
    indices
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use openehr_mapping_core::composition::PositionError;

    use super::fhir_axes;
    use super::render;
    use super::rm_positions;
    use crate::engine::traverse::error::EngineError;
    use crate::resolve::program::target::FhirTarget;
    use crate::tree::element::resolve;
    use crate::tree::path::FhirPath;

    /// Resolves `expression` against the R4 `Condition`.
    fn target(expression: &str) -> FhirTarget {
        let path = FhirPath::from_str(expression).expect("the expression parses");
        let resolved = resolve(&fhir_types::r4::schema::SCHEMAS, "Condition", &path)
            .expect("the expression resolves");
        FhirTarget::new(path, resolved)
    }

    #[test]
    fn two_elements_that_share_a_definition_count_their_instances_apart() {
        // Condition.code and Condition.verificationStatus are both
        // CodeableConcept, so both codings are `CodeableConcept.coding`.
        let code = fhir_axes(&target("$resource.code.coding.code"));
        let status = fhir_axes(&target("$resource.verificationStatus.coding.code"));
        assert_eq!(code, vec![String::from("Condition.code.coding")]);
        assert_eq!(
            status,
            vec![String::from("Condition.verificationStatus.coding")]
        );
    }

    #[test]
    fn an_index_past_u32_is_an_overflow_and_never_a_zero() {
        let past = usize::try_from(u64::from(u32::MAX) + 1).expect("a 64-bit usize");
        let error = rm_positions("site", &[1, past]).expect_err("the index has no position");
        assert!(
            matches!(
                error,
                EngineError::Position {
                    source: PositionError::Overflow,
                    ..
                }
            ),
            "an overflow names itself: {error}"
        );
    }

    #[test]
    fn a_zero_position_is_refused_and_never_dropped() {
        let error = rm_positions("site", &[1, 0]).expect_err("0 is no openEHR position");
        assert!(matches!(
            error,
            EngineError::Position {
                source: PositionError::Zero,
                ..
            }
        ));
        assert_eq!(
            rm_positions("site", &[2, 1])
                .expect("both are positions")
                .iter()
                .map(|position| position.get())
                .collect::<Vec<u32>>(),
            [2, 1]
        );
    }

    #[test]
    fn an_index_prefix_renders_as_the_counter_key() {
        assert_eq!(render(&[]), "");
        assert_eq!(render(&[0, 2]), "0.2");
    }
}
