// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `link` and `participationsFunction` mappings over the reference-model
//! families of a node.

use fhir_types::codec::Value;
use openehr_mapping_core::composition::RmPosition;
use openehr_rm::v1_2::paths::EhrUri;

use crate::engine::family;
use crate::engine::fhir::FhirKind;
use crate::engine::origin::UNKNOWN_SOURCE;
use crate::engine::outcome::SkipReason;
use crate::engine::outcome::Warning;
use crate::model::ast::keyword::Direction;
use crate::resolve::program::mapping::Mapping;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;
use crate::tree::Occurrence;

use crate::tree::element::Table;

use crate::engine::traverse::Binding;
use crate::engine::traverse::PARTICIPATION_SCHEME;
use crate::engine::traverse::Run;
use crate::engine::traverse::error::EngineError;
use crate::engine::traverse::place::openehr_occurrence;
use crate::engine::traverse::tail::tail_segments;
use crate::engine::traverse::tail::unsupported_tail;

impl<T: Table + ?Sized> Run<'_, T> {
    /// Runs a `link` mapping.
    ///
    /// The openEHR side names the `links` of a `LOCATABLE`, and each link is
    /// written as one `_link:i` family on the node (Simplified Formats, the
    /// `LINK` table). The FHIR side is the reference the link targets; a side
    /// that is no reference is the linked composition of
    /// `concept-mappings.adoc` §Linked mappings, which one run does not
    /// produce, and is refused.
    pub(super) fn link(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        meaning: &str,
        link_type: &str,
    ) -> Result<Vec<Binding>, EngineError> {
        let (target, openehr) = Self::family_sides(mapping, "links")?;
        if !matches!(
            FhirKind::at(target.resolved().location()),
            Some(FhirKind::Reference | FhirKind::String)
        ) {
            return Err(EngineError::LinkedComposition {
                mapping: String::from(mapping.name()),
            });
        }
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut bindings = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let binding = Binding {
                        fhir: input.clone(),
                        openehr: parent.openehr.clone(),
                    };
                    let Some(literal) = self.reference_at(mapping, input)? else {
                        self.warnings.push(Warning::Skipped {
                            mapping: String::from(mapping.name()),
                            reason: SkipReason::EmptyReference,
                        });
                        bindings.push(binding);
                        continue;
                    };
                    {
                        if let Err(source) = literal.parse::<EhrUri>() {
                            return Err(EngineError::LinkTarget {
                                mapping: String::from(mapping.name()),
                                target: literal,
                                source,
                            });
                        }
                        let positions = self.family_positions(openehr, &binding.openehr);
                        let index = self.next_family(openehr, &positions, "_link");
                        self.families.extend(family::link(
                            openehr.node(),
                            &positions,
                            index,
                            &family::LinkParts {
                                meaning,
                                link_type,
                                target: &literal,
                            },
                        ));
                    }
                    bindings.push(binding);
                }
                Ok(bindings)
            }
            Direction::OpenehrToFhir => {
                let positions = self.family_positions(openehr, &parent.openehr);
                let Some(node) = self.node_value(mapping.name(), openehr, &positions)? else {
                    return Ok(Vec::new());
                };
                let targets =
                    family::link_targets(&node, meaning, link_type).map_err(|source| {
                        EngineError::Family {
                            mapping: String::from(mapping.name()),
                            source: Box::new(source),
                        }
                    })?;
                let occurrences =
                    vec![openehr_occurrence(mapping.name(), &positions)?; targets.len()];
                let bindings = self.bindings(mapping, parent, &occurrences)?;
                for (binding, literal) in bindings.iter().zip(targets.iter()) {
                    self.write_reference(mapping, binding, literal, None)?;
                }
                Ok(bindings)
            }
        }
    }

    /// Runs a `participationsFunction` mapping.
    ///
    /// "The function in this method is not something that is
    /// auto-transformable. Therefore, it is added to the `with:` statement"
    /// (`concept-mappings.adoc`, §Participation mappings): the FHIR side is
    /// the `Reference` of the participant, the openEHR side the
    /// `other_participations` of an `ENTRY` or the `participations` of the
    /// `EVENT_CONTEXT` (`family::participation_list`), and the function is
    /// the method's. The performer carries the reference as its
    /// `external_ref` and the reference's `display` as its name; the
    /// allocation is our own design.
    pub(super) fn participation(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        function: &str,
    ) -> Result<Vec<Binding>, EngineError> {
        let list = mapping
            .openehr()
            .and_then(|openehr| {
                let tail = tail_segments(openehr);
                family::participation_list(openehr.node().rm_type(), &tail)
            })
            .unwrap_or(family::ENTRY_PARTICIPATIONS);
        let (_target, openehr) = Self::family_sides(mapping, list.attribute)?;
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut bindings = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let binding = Binding {
                        fhir: input.clone(),
                        openehr: parent.openehr.clone(),
                    };
                    let value = self.fhir_value_at(mapping, input)?;
                    let literal = value
                        .as_ref()
                        .and_then(|value| value.get("reference"))
                        .and_then(Value::as_str);
                    let display = value
                        .as_ref()
                        .and_then(|value| value.get("display"))
                        .and_then(Value::as_str);
                    let Some(literal) = literal else {
                        self.warnings.push(Warning::Skipped {
                            mapping: String::from(mapping.name()),
                            reason: SkipReason::EmptyReference,
                        });
                        bindings.push(binding);
                        continue;
                    };
                    let namespace = self.reference_type(literal);
                    let positions = self.family_positions(openehr, &binding.openehr);
                    let index = self.next_family(openehr, &positions, list.family);
                    self.families.extend(family::participation(
                        openehr.node(),
                        list,
                        &positions,
                        index,
                        &family::ParticipationParts {
                            function,
                            name: display,
                            id: literal,
                            id_scheme: PARTICIPATION_SCHEME,
                            id_namespace: namespace.unwrap_or(UNKNOWN_SOURCE),
                        },
                    ));
                    bindings.push(binding);
                }
                Ok(bindings)
            }
            Direction::OpenehrToFhir => {
                let positions = self.family_positions(openehr, &parent.openehr);
                let Some(node) = self.node_value(mapping.name(), openehr, &positions)? else {
                    return Ok(Vec::new());
                };
                let found = family::participations(&node, list, function).map_err(|source| {
                    EngineError::Family {
                        mapping: String::from(mapping.name()),
                        source: Box::new(source),
                    }
                })?;
                let occurrences =
                    vec![openehr_occurrence(mapping.name(), &positions)?; found.len()];
                let bindings = self.bindings(mapping, parent, &occurrences)?;
                for (binding, participant) in bindings.iter().zip(found.iter()) {
                    let Some(ref literal) = participant.id else {
                        continue;
                    };
                    self.write_reference(mapping, binding, literal, participant.name.as_deref())?;
                }
                Ok(bindings)
            }
        }
    }

    /// Returns the two sides of a mapping that writes a reference-model
    /// family, refusing an openEHR side whose tail is not that family.
    fn family_sides<'mapping>(
        mapping: &'mapping Mapping,
        attribute: &str,
    ) -> Result<(&'mapping FhirTarget, &'mapping OpenehrTarget), EngineError> {
        let Some(openehr) = mapping.openehr() else {
            return Err(EngineError::LinkedComposition {
                mapping: String::from(mapping.name()),
            });
        };
        if tail_segments(openehr) != [attribute] {
            return Err(unsupported_tail(mapping, openehr));
        }
        let Some(target) = mapping.fhir() else {
            return Err(EngineError::UnknownElement {
                mapping: String::from(mapping.name()),
                element: String::from(attribute),
            });
        };
        Ok((target, openehr))
    }

    /// Returns the positions a family on the target's node is written at.
    fn family_positions(&self, target: &OpenehrTarget, bound: &[RmPosition]) -> Vec<RmPosition> {
        target
            .occurrences()
            .iter()
            .enumerate()
            .map(|(depth, axis)| {
                bound
                    .get(depth)
                    .copied()
                    .or_else(|| self.pin(axis))
                    .unwrap_or_else(RmPosition::first)
            })
            .collect()
    }

    /// Returns the next free index of one family on one node instance.
    fn next_family(
        &mut self,
        target: &OpenehrTarget,
        positions: &[RmPosition],
        family: &str,
    ) -> usize {
        let key = (
            format!("{family}@{}", target.node().flat_id()),
            positions
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<String>>()
                .join("."),
        );
        let next = self.counters.entry(key).or_insert(0);
        let taken = *next;
        *next = next.saturating_add(1);
        taken
    }

    /// Returns the resource type a literal reference names, when it names one
    /// this FHIR version defines.
    ///
    /// A literal reference is `<type>/<id>`, relative or at the end of an
    /// absolute URL, optionally followed by `/_history/<version>`
    /// (<https://hl7.org/fhir/R4/references.html#literal>).
    fn reference_type<'text>(&self, literal: &'text str) -> Option<&'text str> {
        let segments: Vec<&str> = literal.split('/').collect();
        let end = segments
            .iter()
            .rposition(|segment| *segment == "_history")
            .unwrap_or(segments.len());
        let kind = end.checked_sub(2).and_then(|at| segments.get(at))?;
        self.table.is_resource(kind).then_some(*kind)
    }
}
