// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The resources a run creates: the message resources, the ones a `(Type)`
//! reference creates, and their full urls.

use fhir_types::codec::{Object, Value};
use fhir_types::r4::schema::SCHEMAS;
use sha2::{Digest, Sha256};

use crate::map::notation::{Label, Step};
use crate::map::run::{Base, Identity, Pending, Resource, Run};
use crate::map::{Outcome, RowRef};
use crate::parse::Location;

impl Run<'_> {
    /// Creates, or finds, the resource a `(Type)` reference at the end of
    /// `steps` names, and records the reference to it.
    pub(super) fn reference(
        &mut self,
        base: &Base,
        steps: &[Step],
        repetition: usize,
        at: &Location,
        reference: &RowRef,
    ) -> Option<Base> {
        let target = steps.last()?.reference.clone()?;
        let slots = self.slots(base, steps, repetition, at, reference)?;
        let identity = Identity::Created {
            parent: base.resource,
            slots: slots
                .iter()
                .map(|slot| (slot.name.clone(), slot.key.clone()))
                .collect(),
            label: target.label.clone(),
        };
        let existing = self.resources.iter().position(|resource| {
            resource.identity == identity && resource.type_name == target.resource
        });
        let created = if let Some(index) = existing {
            index
        } else {
            if !SCHEMAS.is_resource(&target.resource) {
                self.outcomes.push(Outcome::UnsupportedShape {
                    at: at.clone(),
                    row: reference.clone(),
                });
                return None;
            }
            let leaf = Base {
                resource: base.resource,
                slots,
            }
            .child("reference", "");
            self.resolve(&leaf, at, reference)?;
            let index = self.create(&target.resource, identity);
            let full_url = self.resources.get(index)?.full_url.clone();
            self.record(
                &leaf,
                Pending::Ready(Value::String(full_url)),
                at,
                reference,
            );
            index
        };
        Some(Base {
            resource: created,
            slots: Vec::new(),
        })
    }

    /// The message resource of `resource_type` and `label` whose occurrence
    /// key is the longest prefix of `key`, created when there is none.
    pub(super) fn message_resource(
        &mut self,
        resource_type: &str,
        label: Option<Label>,
        key: &[usize],
    ) -> usize {
        let found = self
            .resources
            .iter()
            .enumerate()
            .filter_map(|(index, resource)| match &resource.identity {
                Identity::Message {
                    label: existing,
                    key: existing_key,
                } if resource.type_name == resource_type
                    && *existing == label
                    && key.starts_with(existing_key) =>
                {
                    Some((existing_key.len(), index))
                }
                _ => None,
            })
            .max();
        match found {
            Some((_, index)) => index,
            None => self.create(
                resource_type,
                Identity::Message {
                    label,
                    key: key.to_vec(),
                },
            ),
        }
    }

    /// Creates a resource with its derived full url.
    pub(super) fn create(&mut self, resource_type: &str, identity: Identity) -> usize {
        let index = self.resources.len();
        let control = self.parsed.message().control_id().unwrap_or_default();
        self.resources.push(Resource {
            type_name: String::from(resource_type),
            identity,
            full_url: full_url(control, index, resource_type),
            writes: Vec::new(),
            omitted: false,
        });
        index
    }
}

/// An empty document of `resource_type`.
pub(super) fn document_of(resource_type: &str) -> Value {
    let mut object = Object::new();
    object.insert(
        String::from("resourceType"),
        Value::String(String::from(resource_type)),
    );
    Value::Object(object)
}

/// The full url of a created resource: a `urn:uuid` derived from MSH-10, the
/// resource's ordinal in the run and its type.
///
/// No specification governs this: our own design, so the same message maps to
/// the same Bundle and a resent message can be recognized.
pub(super) fn full_url(control: &str, ordinal: usize, resource_type: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(control.as_bytes());
    digest.update([0]);
    digest.update(ordinal.to_string().as_bytes());
    digest.update([0]);
    digest.update(resource_type.as_bytes());
    let hash = digest.finalize();
    let mut bytes = [0u8; 16];
    for (byte, source) in bytes.iter_mut().zip(hash.iter()) {
        *byte = *source;
    }
    let uuid = uuid::Builder::from_custom_bytes(bytes).into_uuid();
    format!("urn:uuid:{uuid}")
}

#[cfg(test)]
mod tests {
    use crate::map::Outcome;
    use crate::map::run::Run;
    use crate::map::run::tests::{corpus, message, shipped, text, values_at, writes_of};
    use crate::parse::Parsed;

    /// A synthetic SIU^S12 for one patient, with the placer and filler order
    /// numbers in SCH-26 and SCH-27.
    fn appointment() -> Parsed {
        let mut sch = vec!["SCH", "APT-46", "APT-F-46", "", "", "", "ROUTINE^Routine^L"];
        sch.extend([""; 9]);
        sch.push("Pullen^Jeri");
        sch.extend(["", "", ""]);
        sch.push("Pullen^Jeri");
        sch.extend(["", "", "", "", ""]);
        sch.extend(["PLC-46^NORTHHOSP", "FIL-46^NORTHHOSP"]);
        let sch = sch.join("|");
        message(&[
            "MSH|^~\\&|SCHED|NORTHHOSP|EHR|SOUTHCLINIC|20260926100000+0200||SIU^S12^SIU_S12|MSG00046|P|2.5.1",
            &sch,
            "PID|1||PAT-0046^^^NORTHHOSP^MR||Doe^Sam^^^^^L||19800101|M",
            "RGS|1",
        ])
    }

    // NOTE: `segment-pid-to-appointment` and `segment-sch-to-appointment` write their `(Type)`
    // rows as R4 Reference.identifier in the supplements, so no second Patient is created.
    #[test]
    fn an_appointment_names_its_patient_and_orders_by_identifier() {
        let parsed = appointment();
        let guide = corpus();
        let mut run = Run::new(&guide, &parsed);
        run.message().expect("the walk runs");
        let patients = |run: &Run<'_>| {
            run.resources
                .iter()
                .filter(|resource| resource.type_name == "Patient")
                .count()
        };
        assert_eq!(patients(&run), 1);
        let refused: Vec<&str> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::NoDatatypeMap { row, .. } => Some(row.source.as_str()),
                _ => None,
            })
            .filter(|source| ["PID-3", "SCH-26", "SCH-27"].contains(source))
            .collect();
        assert_eq!(refused.len(), 4, "{:?}", run.outcomes);
        let supplemented = shipped();
        let mut run = Run::new(&supplemented, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(patients(&run), 1);
        assert_eq!(
            values_at(&run, "Appointment", "basedOn.identifier.value"),
            vec![text("PLC-46"), text("FIL-46")],
            "{:?}",
            writes_of(&run, "Appointment")
        );
        assert_eq!(
            values_at(
                &run,
                "Appointment",
                "extension.valueReference.identifier.value"
            ),
            vec![text("PAT-0046")]
        );
        assert_eq!(
            values_at(&run, "Appointment", "participant.actor.identifier.value"),
            vec![text("PAT-0046")]
        );
        assert!(
            run.resources
                .iter()
                .all(|resource| resource.type_name != "ServiceRequest")
        );
    }
}
