// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The recorded writes: each leaf resolved against its resource and each
//! step given its instance key.

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::tree::element::{Move, resolve};
use fhirconnect::tree::path::FhirPath;

use crate::map::notation::{Label, Step};
use crate::map::run::{Base, Pending, Run, Slot, Write};
use crate::map::{ElementError, Outcome, RowRef};
use crate::parse::Location;

impl Run<'_> {
    /// Records one write against the leaf's resource.
    pub(super) fn record(
        &mut self,
        leaf: &Base,
        value: Pending,
        at: &Location,
        reference: &RowRef,
    ) {
        self.write(leaf, value, at, reference, false);
    }

    /// Records one write, a fallback when `fallback` holds.
    ///
    /// No specification governs this: our own design. A row of the guide
    /// wins over the bridge's fallback for one element: a fallback at an
    /// element a write already fills is dropped, and a row of the guide
    /// drops every fallback at its element. Nothing the message carries is
    /// lost, since the fallback derives from the same value.
    pub(super) fn write(
        &mut self,
        leaf: &Base,
        value: Pending,
        at: &Location,
        reference: &RowRef,
        fallback: bool,
    ) {
        let Some(resource) = self.resources.get(leaf.resource) else {
            return;
        };
        let contested = fallback || resource.writes.iter().any(|write| write.fallback);
        let names: Vec<&str> = leaf.slots.iter().map(|slot| slot.name.as_str()).collect();
        // NOTE: no specification governs this: our own design; a path the element table does not
        // resolve contests nothing, and `build` counts it as `unknown-element` when it applies.
        let flags = if contested {
            repeating(&resource.type_name, &names).ok()
        } else {
            None
        };
        if let Some(flags) = flags {
            let same = |write: &Write| {
                write.slots.len() == leaf.slots.len() && under(&leaf.slots, &flags, &write.slots)
            };
            if fallback && resource.writes.iter().any(same) {
                return;
            }
            if let Some(resource) = self.resources.get_mut(leaf.resource) {
                resource
                    .writes
                    .retain(|write| !(write.fallback && same(write)));
            }
        }
        if let Some(resource) = self.resources.get_mut(leaf.resource) {
            self.pushed = self.pushed.saturating_add(1);
            resource.writes.push(Write {
                slots: leaf.slots.clone(),
                value,
                at: at.clone(),
                row: reference.clone(),
                fallback,
            });
        }
    }

    /// Resolves a leaf's path against its resource type.
    pub(super) fn resolve(
        &mut self,
        leaf: &Base,
        at: &Location,
        reference: &RowRef,
    ) -> Option<fhirconnect::tree::element::Resolved> {
        let resource_type = self.resources.get(leaf.resource)?.type_name.clone();
        let names: Vec<&str> = leaf.slots.iter().map(|slot| slot.name.as_str()).collect();
        match resolve_path(&resource_type, &names) {
            Ok(resolved) => Some(resolved),
            Err(error) => {
                self.outcomes.push(Outcome::UnknownElement {
                    at: at.clone(),
                    row: reference.clone(),
                    error,
                });
                None
            }
        }
    }

    /// The FHIR type a value at `resolved` is rendered as, counting
    /// `untyped-target` when the element has no single type.
    pub(super) fn target_type(
        &mut self,
        resolved: &fhirconnect::tree::element::Resolved,
        at: &Location,
        reference: &RowRef,
    ) -> Option<String> {
        let code = resolved.type_code();
        if code.is_none() {
            self.outcomes.push(Outcome::UntypedTarget {
                at: at.clone(),
                row: reference.clone(),
                element: String::from(resolved.leaf()),
            });
        }
        code.map(String::from)
    }

    /// Extends `base` by `steps`, giving each step its instance key: its label,
    /// and on the first repeating step the source repetition.
    pub(super) fn slots(
        &mut self,
        base: &Base,
        steps: &[Step],
        repetition: usize,
        at: &Location,
        reference: &RowRef,
    ) -> Option<Vec<Slot>> {
        let resource_type = self.resources.get(base.resource)?.type_name.clone();
        let mut names: Vec<&str> = base.slots.iter().map(|slot| slot.name.as_str()).collect();
        names.extend(steps.iter().map(|step| step.name.as_str()));
        let flags = match repeating(&resource_type, &names) {
            Ok(flags) => flags,
            Err(error) => {
                self.outcomes.push(Outcome::UnknownElement {
                    at: at.clone(),
                    row: reference.clone(),
                    error,
                });
                return None;
            }
        };
        let mut slots = base.slots.clone();
        let mut placed_repetition = false;
        for (step, repeats) in steps.iter().zip(flags.iter().skip(base.slots.len())) {
            let mut key = step
                .label
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            if *repeats && !placed_repetition {
                placed_repetition = true;
                key.push('#');
                key.push_str(&repetition.to_string());
            }
            slots.push(Slot {
                name: step.name.clone(),
                key,
            });
        }
        if repetition > 0 && !placed_repetition {
            self.outcomes.push(Outcome::RepetitionDropped {
                at: at.clone(),
                row: reference.clone(),
            });
            return None;
        }
        Some(slots)
    }

    /// The base a data type map's `[k].` prefix names: another instance of
    /// the base's last repeating element for any `k` but 1, or, at the root
    /// of a resource whose map names its own siblings, the `k` sibling.
    pub(super) fn instance(
        &mut self,
        base: &Base,
        instance: Option<&Label>,
        at: &Location,
        reference: &RowRef,
    ) -> Option<Base> {
        if let Some(Label::Number(label)) = instance
            && *label != 1
            && let Some(anchor) = self.family(base, reference)
        {
            return Some(Base {
                resource: self.sibling(anchor, *label),
                slots: Vec::new(),
            });
        }
        let Some(label) = instance.filter(|label| **label != Label::Number(1)) else {
            return Some(base.clone());
        };
        let resource_type = self.resources.get(base.resource)?.type_name.clone();
        let names: Vec<&str> = base.slots.iter().map(|slot| slot.name.as_str()).collect();
        let flags = repeating(&resource_type, &names).unwrap_or_default();
        let Some(last) = flags.iter().rposition(|repeats| *repeats) else {
            self.outcomes.push(Outcome::UnplacedInstance {
                at: at.clone(),
                row: reference.clone(),
            });
            return None;
        };
        let mut moved = base.clone();
        if let Some(slot) = moved.slots.get_mut(last) {
            slot.key.push('/');
            slot.key.push_str(&label.to_string());
        }
        Some(moved)
    }
}

/// Resolves `$resource.<names>` against `resource_type`.
pub(super) fn resolve_path(
    resource_type: &str,
    names: &[&str],
) -> Result<fhirconnect::tree::element::Resolved, ElementError> {
    let text = if names.is_empty() {
        String::from("$resource")
    } else {
        format!("$resource.{}", names.join("."))
    };
    let path = text.parse::<FhirPath>()?;
    Ok(resolve(&SCHEMAS, resource_type, &path)?)
}

/// Whether each step of `names` enters a repeating element.
pub(super) fn repeating(resource_type: &str, names: &[&str]) -> Result<Vec<bool>, ElementError> {
    let mut flags = Vec::with_capacity(names.len());
    let mut seen = 0usize;
    for length in 1..=names.len() {
        let resolved = resolve_path(resource_type, names.get(..length).unwrap_or_default())?;
        let count = resolved
            .moves()
            .iter()
            .filter(|step| match step {
                Move::Member(field) | Move::Extension { field, .. } => field.repeats(),
                _ => false,
            })
            .count();
        flags.push(count > seen);
        seen = count;
    }
    Ok(flags)
}

/// Whether `slots` lie at or under `prefix`, whose repeating steps are
/// `flags`; the key of a step that does not repeat names no instance.
pub(super) fn under(prefix: &[Slot], flags: &[bool], slots: &[Slot]) -> bool {
    slots.len() >= prefix.len()
        && prefix
            .iter()
            .zip(slots)
            .zip(flags)
            .all(|((step, slot), repeats)| {
                step.name == slot.name && (!repeats || step.key == slot.key)
            })
}

#[cfg(test)]
mod tests {
    use super::resolve_path;
    use crate::map::Outcome;
    use crate::map::run::Run;
    use crate::map::run::tests::{corpus, parsed};
    use crate::parse;

    #[test]
    fn a_choice_with_no_alternative_named_is_an_untyped_target() {
        let corpus = corpus();
        let parsed = parsed("NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC");
        let mut run = Run::new(&corpus, &parsed);
        let before = run.outcomes.len();
        let at = parse::Location {
            segment: String::from("OBX"),
            sequence: 1,
            field: Some(5),
            ..parse::Location::default()
        };
        let reference = crate::map::RowRef {
            map: String::from("segment-obx-to-observation"),
            source: String::from("OBX-5"),
            target: String::from("value"),
        };
        let chosen = resolve_path("Observation", &["valueString"]).expect("valueString resolves");
        assert_eq!(
            run.target_type(&chosen, &at, &reference).as_deref(),
            Some("string")
        );
        assert_eq!(run.outcomes.len(), before, "{:?}", run.outcomes);
        let open = resolve_path("Observation", &["value"]).expect("value[x] resolves");
        assert_eq!(run.target_type(&open, &at, &reference), None);
        assert_eq!(
            run.outcomes.get(before..).unwrap_or_default(),
            [Outcome::UntypedTarget {
                at,
                row: reference,
                element: String::from("Observation.value[x]"),
            }]
        );
    }
}
