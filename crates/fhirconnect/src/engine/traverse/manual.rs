// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `manual` entries of a model mapping.

use std::collections::BTreeMap;

use fhir_types::codec::Value;

use crate::engine::condition;
use crate::engine::outcome::SkipReason;
use crate::engine::outcome::Warning;
use crate::engine::rm;
use crate::engine::rm::Carried;
use crate::engine::rm::RmValue;
use crate::model::ast::keyword::Direction;
use crate::resolve::program::manual::Manual;
use crate::resolve::program::mapping::Mapping;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;
use crate::resolve::program::target::Target;
use crate::tree::Occurrence;

use crate::tree::element::Table;
use crate::tree::write::write;

use crate::engine::traverse::Binding;
use crate::engine::traverse::Held;
use crate::engine::traverse::Run;
use crate::engine::traverse::Written;
use crate::engine::traverse::error::EngineError;
use crate::engine::traverse::place::fhir_axes;
use crate::engine::traverse::tail::merge;

impl<T: Table + ?Sized> Run<'_, T> {
    /// Writes one manual entry, merging every path it names into one element.
    pub(super) fn manual(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        if let Some(only) = entry.direction()
            && only != self.direction
        {
            self.warnings.push(Warning::Skipped {
                mapping: format!("{}.{}", mapping.name(), entry.name()),
                reason: SkipReason::Unidirectional,
            });
            return Ok(());
        }
        if !self.manual_admits(mapping, entry, binding)? {
            return Ok(());
        }
        match self.direction {
            Direction::FhirToOpenehr => self.manual_openehr(mapping, entry, binding),
            Direction::OpenehrToFhir => self.manual_fhir(mapping, entry, binding),
        }
    }

    /// Returns whether the input-side conditions admit a manual entry.
    ///
    /// Going out of openEHR the entry's `openehrCondition` is evaluated at the
    /// instance the mapping bound, the rule a mapping's own condition follows
    /// (`basics/Conditions.adoc`, "conditions are always applied on the input
    /// data").
    fn manual_admits(
        &self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<bool, EngineError> {
        let gate = match self.direction {
            Direction::FhirToOpenehr => entry.fhir_condition(),
            Direction::OpenehrToFhir => entry.openehr_condition(),
        };
        let Some(gate) = gate.filter(|gate| condition::runs(gate, self.direction)) else {
            return Ok(true);
        };
        match self.direction {
            Direction::FhirToOpenehr => condition::evaluate(self.table, &self.fhir, gate, false)
                .map(|verdict| verdict.admits_any())
                .map_err(|source| EngineError::Condition {
                    mapping: String::from(mapping.name()),
                    source: Box::new(source),
                }),
            Direction::OpenehrToFhir => self.openehr_holds(mapping.name(), gate, &binding.openehr),
        }
    }

    /// Writes the openEHR paths of one manual entry as one value.
    fn manual_openehr(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let mut merged = serde_json::Map::new();
        let mut node: Option<&OpenehrTarget> = None;
        for path in entry.openehr() {
            let Target::Openehr(ref target) = *path.target() else {
                continue;
            };
            node = Some(target.as_ref());
            let segments: Vec<&str> = target
                .tail()
                .segments
                .iter()
                .map(|segment| segment.attribute.as_str())
                .collect();
            // NOTE: no specification governs this: our own design, `merge` writes a manual
            // value as text, so a tail ending on no string attribute refuses (an empty
            // tail writes the node's `value`).
            let written: &[&str] = if segments.is_empty() {
                &["value"]
            } else {
                &segments
            };
            if rm::carried(target.node().rm_type(), written)
                .is_none_or(|(_, held)| held != Carried::Text)
            {
                return Err(EngineError::UnsupportedTail {
                    mapping: format!("{}.{}", mapping.name(), entry.name()),
                    node: String::from(target.node().aql_path().as_str()),
                    tail: target.tail().to_string(),
                });
            }
            let text = self.manual_text(mapping, entry, path.value())?;
            merge(&mut merged, &segments, text);
        }
        let Some(target) = node else {
            return Ok(());
        };
        let rm_type = target.node().rm_type();
        merged.insert(
            String::from("_type"),
            serde_json::Value::String(String::from(rm_type)),
        );
        let value = RmValue::from_canonical(
            rm_type,
            target.node().aql_path().as_str(),
            &serde_json::Value::Object(merged),
        )
        .map_err(|source| EngineError::Rm {
            mapping: format!("{}.{}", mapping.name(), entry.name()),
            node: String::from(target.node().aql_path().as_str()),
            source: Box::new(source),
        })?;
        self.store(Written {
            flat_id: target.node().flat_id().clone(),
            positions: binding.openehr.clone(),
            value: Held::Value(value),
        });
        Ok(())
    }

    /// Writes the FHIR paths of one manual entry into one element.
    fn manual_fhir(
        &mut self,
        mapping: &Mapping,
        entry: &Manual,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let mut shared: BTreeMap<String, usize> = BTreeMap::new();
        for path in entry.fhir() {
            let Target::Fhir(ref target) = *path.target() else {
                continue;
            };
            let occurrence = self.entry_occurrence(target, binding, &mut shared);
            let element = String::from(target.resolved().leaf());
            let text = Value::String(String::from(self.manual_text(
                mapping,
                entry,
                path.value(),
            )?));
            write(
                self.table,
                &mut self.fhir,
                target.expression(),
                &occurrence,
                text,
            )
            .map_err(|source| EngineError::Write {
                mapping: format!("{}.{}", mapping.name(), entry.name()),
                element,
                source: Box::new(source),
            })?;
        }
        Ok(())
    }

    /// Returns the occurrence one path of a manual entry writes at.
    ///
    /// "Full FHIR or openEHR elements are created from all given paths"
    /// (`types-of-mappings/concept-type/manual.adoc`), so every path of one
    /// entry that passes the same repeating element takes the same instance
    /// of it.
    fn entry_occurrence(
        &mut self,
        target: &FhirTarget,
        binding: &Binding,
        shared: &mut BTreeMap<String, usize>,
    ) -> Occurrence {
        let axes = fhir_axes(target);
        let mut indices: Vec<usize> = binding.fhir.indices().to_vec();
        for axis in axes.iter().skip(indices.len()) {
            let index = if let Some(&taken) = shared.get(axis) {
                taken
            } else {
                let taken = self.next_index(axis, &indices);
                shared.insert(axis.clone(), taken);
                taken
            };
            indices.push(index);
        }
        Occurrence::new(indices)
    }
}
