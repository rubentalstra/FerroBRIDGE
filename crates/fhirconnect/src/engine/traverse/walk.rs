// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The walk over the mappings: the conditions, the input occurrences and the
//! children a mapping runs.

use openehr_mapping_core::composition::RmPosition;

use crate::engine::condition;
use crate::engine::condition::ConditionError;
use crate::engine::condition::Verdict;
use crate::engine::outcome::SkipReason;
use crate::engine::outcome::Warning;
use crate::engine::rm;
use crate::engine::rm::Carried;
use crate::engine::rm::RmValue;
use crate::model::ast::keyword::Direction;
use crate::resolve::derive;
use crate::resolve::program::hierarchy::Preprocessor;
use crate::resolve::program::mapping::Mapping;
use crate::resolve::program::mapping::Method;
use crate::resolve::program::target::Attachment;
use crate::resolve::program::target::OpenehrTarget;
use crate::resolve::program::target::Target;
use crate::tree::Occurrence;

use crate::tree::element::Table;
use crate::tree::read::Selected;
use crate::tree::read::read;

use crate::engine::traverse::Binding;
use crate::engine::traverse::INSTANCE_CEILING;
use crate::engine::traverse::LINK_TYPE;
use crate::engine::traverse::Leaf;
use crate::engine::traverse::Run;
use crate::engine::traverse::error::EngineError;
use crate::engine::traverse::place::fhir_axes;
use crate::engine::traverse::place::openehr_occurrence;
use crate::engine::traverse::place::rm_positions;
use crate::engine::traverse::tail::attribute_at;
use crate::engine::traverse::tail::scalar;
use crate::engine::traverse::tail::tail_segments;
use crate::engine::traverse::tail::unsupported_tail;

impl<T: Table + ?Sized> Run<'_, T> {
    /// Refuses an input the program's own condition does not admit.
    ///
    /// The context's condition is the one the input side carries, the same
    /// rule a mapping's condition follows (`basics/Conditions.adoc`).
    pub(super) fn admits_context(&self) -> Result<(), EngineError> {
        let gate = match self.direction {
            Direction::FhirToOpenehr => self.program.fhir_condition(),
            Direction::OpenehrToFhir => self.program.openehr_condition(),
        };
        let Some(gate) = gate.filter(|gate| condition::runs(gate, self.direction)) else {
            return Ok(());
        };
        let admits = match self.direction {
            Direction::FhirToOpenehr => condition::evaluate(self.table, &self.fhir, gate, false)
                .map(|verdict| verdict.admits_any())
                .map_err(|source| EngineError::Condition {
                    mapping: String::from(self.program.context().as_str()),
                    source: Box::new(source),
                })?,
            Direction::OpenehrToFhir => self.context_holds(gate)?,
        };
        if admits {
            return Ok(());
        }
        Err(EngineError::NotApplicable {
            context: String::from(self.program.context().as_str()),
        })
    }

    /// Returns whether the context's `openehrCondition` holds over the input.
    fn context_holds(
        &self,
        gate: &crate::resolve::program::condition::Condition,
    ) -> Result<bool, EngineError> {
        self.openehr_holds(self.program.context().as_str(), gate, &[])
    }

    /// Runs a list of mappings under one binding, in program order.
    pub(super) fn mappings(
        &mut self,
        mappings: &[Mapping],
        parent: &Binding,
    ) -> Result<(), EngineError> {
        for mapping in mappings {
            self.mapping(mapping, parent)?;
        }
        Ok(())
    }

    /// Runs one mapping under one binding.
    ///
    /// Answers [`Ran::Gated`] when the unidirectional marker or the input
    /// side's condition kept the mapping from running.
    fn mapping(&mut self, mapping: &Mapping, parent: &Binding) -> Result<Ran, EngineError> {
        if let Some(only) = mapping.direction()
            && only != self.direction
        {
            self.warnings.push(Warning::Skipped {
                mapping: String::from(mapping.name()),
                reason: SkipReason::Unidirectional,
            });
            return Ok(Ran::Gated);
        }
        let Some(admitted) = self.admits(mapping, parent)? else {
            return Ok(Ran::Gated);
        };
        let bindings = self.apply(mapping, parent, &admitted)?;
        for binding in &bindings {
            self.children(mapping, binding)?;
        }
        Ok(Ran::Ran)
    }

    /// Returns the input occurrences the conditions admit, `None` for a
    /// mapping a gate closed.
    ///
    /// A condition that filters out every occurrence the input carries closes
    /// the gate as a false one does: the mapping's input is there, and the
    /// condition says it is none this mapping maps.
    fn admits(
        &self,
        mapping: &Mapping,
        parent: &Binding,
    ) -> Result<Option<Vec<Occurrence>>, EngineError> {
        let inputs = self.inputs(mapping, parent)?;
        let Some(gate) = self.input_condition(mapping) else {
            return Ok(Some(inputs));
        };
        match self.direction {
            Direction::FhirToOpenehr => {
                // NOTE: Conditions.adoc §targetRoot, a condition whose target is the
                // `with` element filters that element's occurrences; the compiler
                // decided the attachment over the anchored paths.
                let attached = gate.attachment() == Attachment::Element;
                let verdict = condition::evaluate(self.table, &self.fhir, gate, attached).map_err(
                    |source| EngineError::Condition {
                        mapping: String::from(mapping.name()),
                        source: Box::new(source),
                    },
                )?;
                match verdict {
                    Verdict::Gate(false) => Ok(None),
                    Verdict::Gate(true) => Ok(Some(inputs)),
                    Verdict::Filter(admitted) => {
                        let present = !inputs.is_empty();
                        let kept: Vec<Occurrence> = inputs
                            .into_iter()
                            .filter(|occurrence| admitted.contains(occurrence))
                            .collect();
                        Ok((!kept.is_empty() || !present).then_some(kept))
                    }
                }
            }
            Direction::OpenehrToFhir => {
                let present = !inputs.is_empty();
                let mut admitted = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let positions = Self::positions_at(mapping, &input)?;
                    if self.openehr_holds(mapping.name(), gate, &positions)? {
                        admitted.push(input);
                    }
                }
                Ok((!admitted.is_empty() || !present).then_some(admitted))
            }
        }
    }

    /// Returns whether an `openehrCondition` holds at one instance.
    ///
    /// The condition is evaluated once per instance of the input, so a
    /// `targetRoot` that is the `with` path filters the instances and one
    /// pointing elsewhere answers the same for all of them, which is the plain
    /// true or false `Conditions.adoc` §targetRoot asks for.
    pub(super) fn openehr_holds(
        &self,
        name: &str,
        gate: &crate::resolve::program::condition::Condition,
        instance: &[RmPosition],
    ) -> Result<bool, EngineError> {
        let Some(composition) = self.composition else {
            return Ok(true);
        };
        let mut found = condition::Attributes::default();
        for attribute in gate.attributes() {
            let Target::Openehr(ref target) = *attribute else {
                return Err(EngineError::Condition {
                    mapping: String::from(name),
                    source: Box::new(ConditionError::WrongSide),
                });
            };
            let depth = target.occurrences().len();
            let mut positions: Vec<RmPosition> = instance.iter().take(depth).copied().collect();
            positions.resize(depth, RmPosition::first());
            let read = self
                .index
                .read(composition, target.node(), &positions)
                .map_err(|source| EngineError::Template {
                    mapping: String::from(name),
                    node: String::from(target.node().aql_path().as_str()),
                    source: Box::new(source),
                })?;
            let Some(value) = read else {
                continue;
            };
            let Some(at_tail) = attribute_at(&value, target) else {
                continue;
            };
            found.present = found.present.saturating_add(1);
            found.types.push(String::from(target.node().rm_type()));
            if let Some(text) = scalar(at_tail) {
                found.values.push(text);
            }
        }
        Ok(condition::decide(gate, &found))
    }

    /// Returns the positions one input occurrence names, as it was read.
    fn positions_at(mapping: &Mapping, input: &Occurrence) -> Result<Vec<RmPosition>, EngineError> {
        rm_positions(mapping.name(), input.indices())
    }

    /// Returns the condition the direction evaluates, if the mapping carries
    /// one.
    fn input_condition<'mapping>(
        &self,
        mapping: &'mapping Mapping,
    ) -> Option<&'mapping crate::resolve::program::condition::Condition> {
        match self.direction {
            Direction::FhirToOpenehr => mapping.fhir_condition(),
            Direction::OpenehrToFhir => mapping.openehr_condition(),
        }
        .filter(|gate| condition::runs(gate, self.direction))
    }

    /// Returns the occurrences of the input side, under the parent binding.
    fn inputs(&self, mapping: &Mapping, parent: &Binding) -> Result<Vec<Occurrence>, EngineError> {
        match self.direction {
            Direction::FhirToOpenehr => {
                let Some(input) = mapping.fhir() else {
                    return Ok(vec![parent.fhir.clone()]);
                };
                let matches =
                    read(self.table, &self.fhir, input.expression()).map_err(|source| {
                        EngineError::Read {
                            mapping: String::from(mapping.name()),
                            expression: String::from(input.expression().as_str()),
                            source: Box::new(source),
                        }
                    })?;
                let pinned = self
                    .fhir_pin
                    .as_ref()
                    .filter(|&(axes, _)| fhir_axes(input).starts_with(axes))
                    .map(|(_, occurrence)| occurrence);
                let mut occurrences = Vec::new();
                for matched in matches {
                    if !matched
                        .occurrence()
                        .indices()
                        .starts_with(parent.fhir.indices())
                    {
                        continue;
                    }
                    if pinned.is_some_and(|pin| {
                        !matched.occurrence().indices().starts_with(pin.indices())
                    }) {
                        continue;
                    }
                    if let Selected::Deferred(_) = *matched.selected() {
                        return Err(EngineError::DeferredReference {
                            mapping: String::from(mapping.name()),
                            expression: String::from(input.expression().as_str()),
                        });
                    }
                    occurrences.push(matched.occurrence().clone());
                }
                Ok(occurrences)
            }
            Direction::OpenehrToFhir => {
                // NOTE: no specification governs this: our own design, a
                // mapping with no openEHR side reads the instance its parent
                // bound, so its one input is the parent's openEHR occurrence.
                let Some(input) = mapping.openehr() else {
                    return Ok(vec![openehr_occurrence(mapping.name(), &parent.openehr)?]);
                };
                let instances = self.instances(mapping.name(), input, parent)?;
                let mut occurrences = Vec::with_capacity(instances.len());
                for positions in instances {
                    occurrences.push(openehr_occurrence(mapping.name(), &positions)?);
                }
                Ok(occurrences)
            }
        }
    }

    /// Returns the instances of the input node under the parent binding.
    ///
    /// An axis the parent bound or a split pinned keeps its instance. The
    /// composition carries no instance count, so the probe of the first free
    /// axis stops at the first instance the read does not find.
    pub(super) fn instances(
        &self,
        name: &str,
        input: &OpenehrTarget,
        parent: &Binding,
    ) -> Result<Vec<Vec<RmPosition>>, EngineError> {
        let axes = input.occurrences();
        let depth = axes.len();
        let bound = parent.openehr.len().min(depth);
        let mut prefix: Vec<RmPosition> = parent.openehr.iter().take(bound).copied().collect();
        while let Some(pinned) = axes.get(prefix.len()).and_then(|axis| self.pin(axis)) {
            prefix.push(pinned);
        }
        if prefix.len() >= depth {
            prefix.truncate(depth);
            // NOTE: no specification governs this: our own design, a node the
            // composition does not hold is no input occurrence, so nothing below it runs.
            if self.node_value(name, input, &prefix)?.is_none() {
                return Ok(Vec::new());
            }
            return Ok(vec![prefix]);
        }
        let mut found = Vec::new();
        let mut instance = 1u32;
        while instance <= INSTANCE_CEILING {
            let mut positions = prefix.clone();
            positions.push(
                RmPosition::new(instance).map_err(|source| EngineError::Position {
                    mapping: String::from(name),
                    source,
                })?,
            );
            for axis in axes.iter().skip(positions.len()) {
                positions.push(self.pin(axis).unwrap_or_else(RmPosition::first));
            }
            if self.node_value(name, input, &positions)?.is_none() {
                break;
            }
            found.push(positions);
            instance = instance.saturating_add(1);
        }
        Ok(found)
    }

    /// Returns the canonical JSON one instance of the target's node holds.
    pub(super) fn node_value(
        &self,
        name: &str,
        input: &OpenehrTarget,
        positions: &[RmPosition],
    ) -> Result<Option<serde_json::Value>, EngineError> {
        let Some(composition) = self.composition else {
            return Ok(None);
        };
        let node = input.node();
        self.index
            .read(composition, node, positions)
            .map_err(|source| EngineError::Template {
                mapping: String::from(name),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            })
    }

    /// Returns the value one instance of the input holds at the end of its
    /// path, below the node when the path names a tail.
    ///
    /// The tail is walked by attribute name over the node's canonical JSON,
    /// and the leaf is read as the class the resolver recorded for it, so the
    /// tail's class selects the cell. A tail no FLAT part of the node's class
    /// carries is refused here as it is on the write side.
    pub(super) fn value_at(
        &self,
        mapping: &Mapping,
        input: &OpenehrTarget,
        positions: &[RmPosition],
    ) -> Result<Option<Leaf>, EngineError> {
        let Some(value) = self.node_value(mapping.name(), input, positions)? else {
            return Ok(None);
        };
        let node = input.node();
        if !input.tail().segments.is_empty() {
            let segments = tail_segments(input);
            let (_class, carried) = rm::carried(node.rm_type(), &segments)
                .ok_or_else(|| unsupported_tail(mapping, input))?;
            let Some(found) = attribute_at(&value, input) else {
                return Ok(None);
            };
            if !matches!(carried, Carried::Value | Carried::Family) {
                return Ok(scalar(found).map(Leaf::Scalar));
            }
            let leaf = input
                .leaf_class()
                .ok_or_else(|| unsupported_tail(mapping, input))?;
            return RmValue::from_canonical(leaf, node.aql_path().as_str(), found)
                .map(|value| Some(Leaf::Value(Box::new(value))))
                .map_err(|source| EngineError::Rm {
                    mapping: String::from(mapping.name()),
                    node: String::from(node.aql_path().as_str()),
                    source: Box::new(source),
                });
        }
        RmValue::from_canonical(node.rm_type(), node.aql_path().as_str(), &value)
            .map(|value| Some(Leaf::Value(Box::new(value))))
            .map_err(|source| EngineError::Rm {
                mapping: String::from(mapping.name()),
                node: String::from(node.aql_path().as_str()),
                source: Box::new(source),
            })
    }

    /// Applies one mapping's method to every admitted input occurrence.
    ///
    /// Returns the binding each occurrence produced, which is what a
    /// `followedBy` child runs under.
    fn apply(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
    ) -> Result<Vec<Binding>, EngineError> {
        match *mapping.method() {
            Method::Value => self.values(mapping, parent, inputs),
            Method::Slot {
                ref model,
                ref preprocessors,
                ref mappings,
            } => {
                let name = String::from(model.as_str());
                if self.chain.contains(&name) {
                    return Err(EngineError::SlotCycle {
                        chain: self.chain.join(" -> "),
                        model: name,
                    });
                }
                let bindings = self.bindings(mapping, parent, inputs)?;
                let mut admitted = Vec::with_capacity(bindings.len());
                for binding in &bindings {
                    if self.slot_admits(mapping, preprocessors, binding)? {
                        admitted.push(binding);
                    } else {
                        self.warnings.push(Warning::Skipped {
                            mapping: String::from(mapping.name()),
                            reason: SkipReason::PreprocessorGate {
                                model: name.clone(),
                            },
                        });
                    }
                }
                // NOTE: HierarchyMappings.adoc, the hierarchy lives in the
                // preprocessor of the file it belongs to, so a slotted file
                // splits the slotted mappings under each slot binding.
                let hierarchy = preprocessors
                    .iter()
                    .find(|preprocessor| preprocessor.model() == model)
                    .and_then(Preprocessor::hierarchy);
                self.chain.push(name);
                for binding in admitted {
                    self.split_or_run(mappings, hierarchy, model.as_str(), binding)?;
                }
                self.chain.pop();
                Ok(bindings)
            }
            Method::Programmed { ref code } => self.programmed(mapping, parent, inputs, code),
            Method::Reference {
                ref resource,
                ref mappings,
            } => self.reference(mapping, parent, inputs, resource, mappings),
            Method::Link {
                ref meaning,
                ref link_type,
            } => {
                let meaning = meaning.as_deref().unwrap_or(mapping.name());
                let link_type = link_type.as_deref().unwrap_or(LINK_TYPE);
                self.link(mapping, parent, inputs, meaning, link_type)
            }
            Method::Participation { ref function } => {
                self.participation(mapping, parent, inputs, function)
            }
        }
    }

    /// Returns whether every preprocessor gate of a slotted file admits the
    /// input at `binding`.
    ///
    /// A file's preprocessor condition "defines that the mapping file is only
    /// executed if the given condition is met" (`basics/Conditions.adoc`,
    /// §Conditions in the preprocessor), read on the input side like every
    /// condition, so a closed gate skips the slotted mappings for that
    /// occurrence and the skip is a recorded outcome.
    fn slot_admits(
        &self,
        mapping: &Mapping,
        preprocessors: &[Preprocessor],
        binding: &Binding,
    ) -> Result<bool, EngineError> {
        for preprocessor in preprocessors {
            let gate = match self.direction {
                Direction::FhirToOpenehr => preprocessor.fhir_condition(),
                Direction::OpenehrToFhir => preprocessor.openehr_condition(),
            };
            let Some(gate) = gate.filter(|gate| condition::runs(gate, self.direction)) else {
                continue;
            };
            let holds = match self.direction {
                Direction::FhirToOpenehr => {
                    condition::evaluate(self.table, &self.fhir, gate, false)
                        .map(|verdict| verdict.admits_any())
                        .map_err(|source| EngineError::Condition {
                            mapping: String::from(mapping.name()),
                            source: Box::new(source),
                        })?
                }
                Direction::OpenehrToFhir => {
                    self.openehr_holds(preprocessor.model().as_str(), gate, &binding.openehr)?
                }
            };
            if !holds {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Runs the `followedBy` children of one mapping under its binding.
    ///
    /// "If we have a parent node with a `1..1` cardinality and a child node
    /// with a `1..1` cardinality, the mapping should fail if the child is not
    /// provided" (`engine/Fail.adoc`), so a child the template requires and
    /// the input does not carry refuses the unit. A child its own condition
    /// closed was provided and is none the mapping maps, so it is skipped,
    /// and a structural node counts as provided once any value below it is
    /// written.
    fn children(&mut self, mapping: &Mapping, binding: &Binding) -> Result<(), EngineError> {
        for child in mapping.followed_by() {
            let ran = self.mapping(child, binding)?;
            if self.direction != Direction::FhirToOpenehr || ran == Ran::Gated {
                continue;
            }
            let Some(target) = child.openehr() else {
                continue;
            };
            if target.node().min().is_none_or(|min| min < 1) {
                continue;
            }
            if !self.provided(target) {
                return Err(EngineError::MissingRequired {
                    mapping: String::from(child.name()),
                    node: String::from(target.node().aql_path().as_str()),
                });
            }
        }
        Ok(())
    }

    /// Returns whether the run wrote the node a required child names.
    ///
    /// A structural node holds no value of its own, so it is provided once
    /// the run wrote any value at or below it.
    fn provided(&self, target: &OpenehrTarget) -> bool {
        let node = target.node().flat_id();
        if !derive::is_structural(target.node().rm_type()) {
            return self.written.iter().any(|written| written.flat_id == *node);
        }
        let below = format!("{}/", node.as_str());
        self.written
            .iter()
            .any(|written| written.flat_id == *node || written.flat_id.as_str().starts_with(&below))
            || self.families.iter().any(|family| {
                family
                    .flat_id()
                    .is_some_and(|flat_id| flat_id == node || flat_id.as_str().starts_with(&below))
            })
    }
}

/// Whether a mapping ran, or its input side kept it from running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ran {
    /// The mapping ran over the occurrences its input admitted.
    Ran,
    /// The unidirectional marker or the input side's condition kept it from
    /// running.
    Gated,
}
