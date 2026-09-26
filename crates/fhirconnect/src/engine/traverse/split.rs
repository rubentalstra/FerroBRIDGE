// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `hierarchy.split` groups, each run with its axes pinned.

use openehr_mapping_core::composition::FlatIndex;
use openehr_mapping_core::composition::PositionError;
use openehr_mapping_core::composition::RmPosition;
use openehr_mapping_core::index::paths::FlatId;

use crate::engine::outcome::SkipReason;
use crate::engine::outcome::Warning;
use crate::model::ast::keyword::Direction;
use crate::resolve::program::hierarchy::Create;
use crate::resolve::program::hierarchy::Hierarchy;
use crate::resolve::program::hierarchy::Split;
use crate::resolve::program::mapping::Mapping;
use crate::resolve::program::target::OpenehrTarget;
use crate::resolve::program::target::Target;
use crate::tree::Occurrence;

use crate::tree::element::Table;
use crate::tree::read::read;

use crate::engine::traverse::Binding;
use crate::engine::traverse::Run;
use crate::engine::traverse::error::EngineError;
use crate::engine::traverse::error::SplitRefusal;
use crate::engine::traverse::place::fhir_axes;
use crate::engine::traverse::tail::attribute_at;
use crate::engine::traverse::tail::lexical;
use crate::engine::traverse::tail::scalar;

impl<T: Table + ?Sized> Run<'_, T> {
    /// Runs a list of mappings, split by the hierarchy of the file they came
    /// from when it splits the direction's output.
    ///
    /// `split.fhir` creates FHIR resources, so it runs going out of openEHR,
    /// and `split.openehr` creates openEHR elements, so it runs going into
    /// openEHR (`types-of-mappings/concept-type/HierarchyMappings.adoc`,
    /// §split).
    pub(super) fn split_or_run(
        &mut self,
        mappings: &[Mapping],
        hierarchy: Option<&Hierarchy>,
        model: &str,
        parent: &Binding,
    ) -> Result<(), EngineError> {
        let Some(hierarchy) = hierarchy else {
            return self.mappings(mappings, parent);
        };
        match self.direction {
            Direction::OpenehrToFhir => match hierarchy.split_fhir() {
                Some(split) => self.split_resources(mappings, hierarchy, split, model, parent),
                None => self.mappings(mappings, parent),
            },
            Direction::FhirToOpenehr => match hierarchy.split_openehr() {
                Some(split) => self.split_elements(mappings, hierarchy, split, model, parent),
                None => self.mappings(mappings, parent),
            },
        }
    }

    /// Creates one FHIR resource per occurrence of the split's openEHR path
    /// and distinct `unique` tuple.
    ///
    /// "For each occurrence of `$archetype/data[at0001]/events[at0002]`, the
    /// `split` is executed", and "one could imagine this process as cloning
    /// the composition with one event each"
    /// (`types-of-mappings/concept-type/HierarchyMappings.adoc`, §split): the
    /// whole mapping set runs once per group with the split node pinned to
    /// the group's occurrence, so content outside the node reaches every
    /// resource. The first group fills the run's own resource and every
    /// further one is a created resource with an id from the identity sink.
    /// A split with no `unique` key makes every occurrence its own group.
    fn split_resources(
        &mut self,
        mappings: &[Mapping],
        hierarchy: &Hierarchy,
        split: &Split,
        model: &str,
        parent: &Binding,
    ) -> Result<(), EngineError> {
        let refuse = |reason: SplitRefusal| EngineError::Split {
            model: String::from(model),
            reason,
        };
        match split.create() {
            Some(Create::Resource) => {}
            Some(create) => return Err(refuse(SplitRefusal::WrongSide { create })),
            None => return Err(refuse(SplitRefusal::NoCreate)),
        }
        if split.path().is_some() {
            return Err(refuse(SplitRefusal::ResourcePath));
        }
        let with = hierarchy
            .openehr()
            .ok_or_else(|| refuse(SplitRefusal::NoWith))?;
        let mut groups: Vec<(Vec<String>, Vec<Vec<RmPosition>>)> = Vec::new();
        for positions in self.instances(model, with, parent)? {
            let tuple = self.unique_openehr(model, split.unique(), &positions)?;
            match groups
                .iter_mut()
                .find(|group| !split.unique().is_empty() && group.0 == tuple)
            {
                Some(group) => group.1.push(positions),
                None => groups.push((tuple, vec![positions])),
            }
        }
        if groups.is_empty() {
            return self.mappings(mappings, parent);
        }
        let base = self.fhir.clone();
        let start = self.warnings.len();
        for (index, (tuple, members)) in groups.into_iter().enumerate() {
            if index == 0 {
                self.run_pinned(mappings, parent, with, &members)?;
                continue;
            }
            let outer = core::mem::replace(&mut self.fhir, base.clone());
            let counters = core::mem::take(&mut self.counters);
            let ran = self.run_pinned(mappings, parent, with, &members);
            self.counters = counters;
            let mut created = core::mem::replace(&mut self.fhir, outer);
            ran?;
            let occurrence = members
                .first()
                .map(|positions| positions.iter().map(|position| position.get()).collect())
                .unwrap_or_default();
            let request = self
                .identity_request(self.program.resource().as_str(), model)
                .with_occurrence(occurrence)
                .with_unique(tuple);
            self.identify(model, &mut created, &request)?;
            self.created.push(created);
        }
        self.settle_split_warnings(start);
        Ok(())
    }

    /// Runs a mapping set once per member, with the split node pinned to it.
    fn run_pinned(
        &mut self,
        mappings: &[Mapping],
        parent: &Binding,
        with: &OpenehrTarget,
        members: &[Vec<RmPosition>],
    ) -> Result<(), EngineError> {
        for positions in members {
            let pins: Vec<(FlatId, RmPosition)> = with
                .occurrences()
                .iter()
                .cloned()
                .zip(positions.iter().copied())
                .collect();
            let held = core::mem::replace(&mut self.pins, pins);
            let ran = self.mappings(mappings, parent);
            self.pins = held;
            ran?;
        }
        Ok(())
    }

    /// Creates one openEHR element per occurrence of the split's FHIR path
    /// and distinct `unique` tuple.
    ///
    /// "For each `dosage` with a different `route` and/or `timing.event`,
    /// create a new `EVENT` in openEHR" (`HierarchyMappings.adoc`, §Hierarchy
    /// and unique values): each group pins the FHIR path to its occurrences
    /// and the created node to a fresh instance, so every mapping under the
    /// node writes into the group's own element.
    fn split_elements(
        &mut self,
        mappings: &[Mapping],
        hierarchy: &Hierarchy,
        split: &Split,
        model: &str,
        parent: &Binding,
    ) -> Result<(), EngineError> {
        let refuse = |reason: SplitRefusal| EngineError::Split {
            model: String::from(model),
            reason,
        };
        match split.create() {
            Some(Create::Event | Create::Archetype) => {}
            Some(create) => return Err(refuse(SplitRefusal::WrongSide { create })),
            None => return Err(refuse(SplitRefusal::NoCreate)),
        }
        let with = hierarchy
            .fhir()
            .ok_or_else(|| refuse(SplitRefusal::NoWith))?;
        let node = match split.path() {
            Some(Target::Openehr(target)) => target.as_ref(),
            Some(Target::Fhir(_)) | None => hierarchy
                .openehr()
                .ok_or_else(|| refuse(SplitRefusal::NoWith))?,
        };
        let axes = node.occurrences();
        if axes.last() != Some(node.node().flat_id()) {
            return Err(refuse(SplitRefusal::NotRepeating {
                node: String::from(node.node().aql_path().as_str()),
            }));
        }
        let matches = read(self.table, &self.fhir, with.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(model),
                expression: String::from(with.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        let mut groups: Vec<(Vec<String>, Vec<Occurrence>)> = Vec::new();
        for matched in matches {
            if !matched
                .occurrence()
                .indices()
                .starts_with(parent.fhir.indices())
            {
                continue;
            }
            let occurrence = matched.occurrence().clone();
            let tuple = self.unique_fhir(model, split.unique(), &occurrence)?;
            match groups
                .iter_mut()
                .find(|group| !split.unique().is_empty() && group.0 == tuple)
            {
                Some(group) => group.1.push(occurrence),
                None => groups.push((tuple, vec![occurrence])),
            }
        }
        if groups.is_empty() {
            return self.mappings(mappings, parent);
        }
        let with_axes = fhir_axes(with);
        let start = self.warnings.len();
        for (index, (_tuple, members)) in groups.into_iter().enumerate() {
            let instance = u32::try_from(index)
                .ok()
                .and_then(|index| RmPosition::try_from(FlatIndex::new(index)).ok())
                .ok_or(EngineError::Position {
                    mapping: String::from(model),
                    source: PositionError::Overflow,
                })?;
            let mut pins: Vec<(FlatId, RmPosition)> = Vec::with_capacity(axes.len());
            for (depth, axis) in axes.iter().enumerate() {
                let position = if depth.saturating_add(1) == axes.len() {
                    instance
                } else {
                    parent
                        .openehr
                        .get(depth)
                        .copied()
                        .unwrap_or_else(RmPosition::first)
                };
                pins.push((axis.clone(), position));
            }
            for occurrence in members {
                let held = core::mem::replace(&mut self.pins, pins.clone());
                let held_fhir = self.fhir_pin.replace((with_axes.clone(), occurrence));
                let ran = self.mappings(mappings, parent);
                self.pins = held;
                self.fhir_pin = held_fhir;
                ran?;
            }
        }
        self.settle_split_warnings(start);
        Ok(())
    }

    /// Keeps one of each warning a split's groups declared alike when the
    /// warning does not depend on the group.
    ///
    /// A mapping skipped for its `unidirectional` marker is skipped for every
    /// group the same way, so the split declares that loss once; a warning
    /// that depends on the group's data (a dropped occurrence, a one-way row,
    /// an unresolved reference) stays once per group. No specification governs
    /// this: our own design.
    fn settle_split_warnings(&mut self, start: usize) {
        settle(&mut self.warnings, start);
    }

    /// Returns the `unique` tuple one openEHR occurrence carries.
    ///
    /// "The path in the `unique:` key relates to the `with:` method"
    /// (`HierarchyMappings.adoc`), so each value is read at the occurrence's
    /// own instance, and an absent value is the empty text.
    fn unique_openehr(
        &self,
        model: &str,
        unique: &[Target],
        positions: &[RmPosition],
    ) -> Result<Vec<String>, EngineError> {
        let Some(composition) = self.composition else {
            return Ok(Vec::new());
        };
        let mut tuple = Vec::with_capacity(unique.len());
        for target in unique {
            let Target::Openehr(ref target) = *target else {
                tuple.push(String::new());
                continue;
            };
            let depth = target.occurrences().len();
            let mut at: Vec<RmPosition> = positions.iter().take(depth).copied().collect();
            at.resize(depth, RmPosition::first());
            let read = self
                .index
                .read(composition, target.node(), &at)
                .map_err(|source| EngineError::Template {
                    mapping: String::from(model),
                    node: String::from(target.node().aql_path().as_str()),
                    source: Box::new(source),
                })?;
            // NOTE: no specification governs this: our own design, a data value
            // compares by the text of its `value`, which is what a reader of
            // the element sees, and any other structure by its JSON.
            let text = read
                .as_ref()
                .and_then(|value| attribute_at(value, target))
                .map(|value| {
                    scalar(value)
                        .or_else(|| value.get("value").and_then(scalar))
                        .unwrap_or_else(|| value.to_string())
                })
                .unwrap_or_default();
            tuple.push(text);
        }
        Ok(tuple)
    }

    /// Returns the `unique` tuple one FHIR occurrence carries.
    fn unique_fhir(
        &self,
        model: &str,
        unique: &[Target],
        occurrence: &Occurrence,
    ) -> Result<Vec<String>, EngineError> {
        let mut tuple = Vec::with_capacity(unique.len());
        for target in unique {
            let Target::Fhir(ref target) = *target else {
                tuple.push(String::new());
                continue;
            };
            let matches = read(self.table, &self.fhir, target.expression()).map_err(|source| {
                EngineError::Read {
                    mapping: String::from(model),
                    expression: String::from(target.expression().as_str()),
                    source: Box::new(source),
                }
            })?;
            let mut texts = Vec::new();
            for matched in &matches {
                if !matched
                    .occurrence()
                    .indices()
                    .starts_with(occurrence.indices())
                {
                    continue;
                }
                if let Some(value) = matched.value() {
                    texts.push(lexical(value));
                }
            }
            tuple.push(texts.join("|"));
        }
        Ok(tuple)
    }

    /// Returns the openEHR pin of one axis, when a split pinned it.
    pub(super) fn pin(&self, axis: &FlatId) -> Option<RmPosition> {
        self.pin_named(axis.as_str())
    }

    /// Returns the openEHR pin of one axis, by its flat id.
    ///
    /// Only an openEHR axis takes a pin; a FHIR output axis is a FHIR path,
    /// which no flat id equals.
    pub(super) fn pin_named(&self, axis: &str) -> Option<RmPosition> {
        self.pins
            .iter()
            .find(|pinned| pinned.0.as_str() == axis)
            .map(|pinned| pinned.1)
    }
}

/// Drops each group-independent warning declared from `start` on that an
/// earlier warning from `start` on already declares.
fn settle(warnings: &mut Vec<Warning>, start: usize) {
    let start = start.min(warnings.len());
    let declared = warnings.split_off(start);
    for warning in declared {
        let alike = matches!(
            warning,
            Warning::Skipped {
                reason: SkipReason::Unidirectional,
                ..
            }
        );
        let seen = warnings
            .get(start..)
            .is_some_and(|kept| kept.contains(&warning));
        if alike && seen {
            continue;
        }
        warnings.push(warning);
    }
}

#[cfg(test)]
mod tests {
    use super::settle;
    use crate::engine::outcome::SkipReason;
    use crate::engine::outcome::Warning;

    #[test]
    fn a_split_declares_a_group_independent_skip_once_and_keeps_the_rest() {
        let skip = Warning::Skipped {
            mapping: String::from("subject"),
            reason: SkipReason::Unidirectional,
        };
        let dropped = Warning::LastOfMany {
            path: String::from("/content"),
            dropped: 1,
        };
        let before = Warning::Defaulted {
            field: String::from("/composer"),
        };
        let mut warnings = vec![
            before.clone(),
            skip.clone(),
            dropped.clone(),
            skip.clone(),
            dropped.clone(),
        ];
        settle(&mut warnings, 1);
        assert_eq!(warnings, [before, skip, dropped.clone(), dropped]);
    }
}
