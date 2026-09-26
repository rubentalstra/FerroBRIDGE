// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `MessageHeader` endpoints the bridge fills from the facility fields
//! and from an `HD` no row of the guide writes.

use fhir_types::codec::Value;

use crate::map::convert;
use crate::map::corpus::Map;
use crate::map::notation::{self, Target};
use crate::map::run::datum::Datum;
use crate::map::run::segment::row_ref;
use crate::map::run::write::repeating;
use crate::map::run::write::resolve_path;
use crate::map::run::write::under;
use crate::map::run::{Base, FACILITY_ENDPOINTS, Pending, Run, Scope};
use crate::map::{Outcome, RowRef};
use crate::parse::{Field, Location};

impl<'a> Run<'a> {
    /// Writes each `MessageHeader` endpoint of [`FACILITY_ENDPOINTS`] whose
    /// fields are empty from the first valued repetition of its facility
    /// field, by the rule of [`Run::endpoint`], and counts it.
    ///
    /// The guide's own rows for those empty fields write a data-absent-reason
    /// extension on the endpoint ([`Run::absence`]); the facility's value
    /// replaces those writes, so an endpoint never carries both.
    pub(super) fn facility_endpoints(&mut self, scope: &Scope<'a>, map: &'a Map, base: &Base) {
        let Some(segment) = scope.segment() else {
            return;
        };
        let header = self
            .resources
            .get(base.resource)
            .is_some_and(|resource| resource.type_name == "MessageHeader");
        if segment.id() != "MSH" || !header || !base.slots.is_empty() {
            return;
        }
        for (target, fields, facility) in FACILITY_ENDPOINTS {
            let valued = |position: usize| segment.field(position).is_some_and(Field::is_valued);
            if fields.into_iter().any(valued) {
                continue;
            }
            let Some((repetition, value)) = segment.field(facility).and_then(|field| {
                field
                    .repetitions()
                    .iter()
                    .enumerate()
                    .find(|(_, value)| value.is_valued())
            }) else {
                continue;
            };
            let source = format!("MSH-{facility}");
            let Some(row) = map.rows.iter().find(|row| row.source == source) else {
                continue;
            };
            let Ok(Target::Path { steps, .. }) = notation::parse(target) else {
                continue;
            };
            let reference = row_ref(map, row);
            let at = Location {
                repetition: Some(repetition.saturating_add(1)),
                ..scope.at.clone().with_field(facility)
            };
            let Some(slots) = self.slots(base, &steps, 0, &at, &reference) else {
                continue;
            };
            let leaf = Base {
                resource: base.resource,
                slots,
            };
            let names: Vec<&str> = leaf.slots.iter().map(|slot| slot.name.as_str()).collect();
            let element = format!("MessageHeader.{}", names.join("."));
            let flags = match repeating("MessageHeader", &names) {
                Ok(flags) => flags,
                Err(error) => {
                    self.outcomes.push(Outcome::UnknownElement {
                        at,
                        row: reference,
                        error,
                    });
                    continue;
                }
            };
            // NOTE: `segment-msh-to-messageheader`, the MSH-24 valueCode row's comment: the
            // implementer assigns a known value or the data-absent-reason, so the value replaces it.
            if let Some(resource) = self.resources.get_mut(leaf.resource) {
                resource
                    .writes
                    .retain(|write| !under(&leaf.slots, &flags, &write.slots));
            }
            self.endpoint(Datum::Repetition(value), &leaf, &at, &reference);
            self.outcomes.push(Outcome::FacilityEndpoint {
                at,
                row: reference,
                element,
            });
        }
    }

    /// Writes a v2 `HD` into a FHIR `url` element, with its namespace ID in
    /// the `name` element beside it when there is one.
    ///
    /// The url is the derived form of [`convert::endpoint`], written as a
    /// fallback ([`Run::write`]): where a row of the guide writes the
    /// endpoint, as `datatype-hd-endpoint-to-messageheader-source` does for a
    /// typed universal ID, that row's value stands.
    pub(super) fn endpoint(
        &mut self,
        datum: Datum<'a>,
        leaf: &Base,
        at: &Location,
        reference: &RowRef,
    ) {
        let component = |position| datum.child(position).text();
        let url = match convert::endpoint(component(1), component(2), component(3)) {
            Ok(url) => url,
            Err(error) => {
                self.outcomes.push(Outcome::Unconvertible {
                    at: at.clone(),
                    row: reference.clone(),
                    error,
                });
                return;
            }
        };
        self.write(
            leaf,
            Pending::Ready(Value::String(url)),
            at,
            reference,
            true,
        );
        let Some(namespace) = component(1) else {
            return;
        };
        let mut slots = leaf.slots.clone();
        slots.pop();
        let name = Base {
            resource: leaf.resource,
            slots,
        }
        .child("name", "");
        let resource_type = self
            .resources
            .get(leaf.resource)
            .map(|resource| resource.type_name.clone())
            .unwrap_or_default();
        let names: Vec<&str> = name.slots.iter().map(|slot| slot.name.as_str()).collect();
        // NOTE: no specification governs this: our own design; both HD endpoint maps of the
        // guide write HD.1 into the `name` beside the endpoint, so the namespace ID stays.
        let holds_name = resolve_path(&resource_type, &names)
            .is_ok_and(|resolved| resolved.type_code() == Some("string"));
        if holds_name {
            let value = Value::String(String::from(namespace));
            self.write(&name, Pending::Ready(value), at, reference, true);
        }
    }
}
