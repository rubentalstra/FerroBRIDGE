// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A value written through a row: the target steps, the assignment, the
//! primitive, the complex element and the table translation.

use std::collections::BTreeSet;

use fhir_types::codec::Value;

use crate::map::convert;
use crate::map::corpus::{Assignment, Kind, Map, MappedVia, Row};
use crate::map::notation::{Step, Target};
use crate::map::run::datum::Datum;
use crate::map::run::write::resolve_path;
use crate::map::run::{
    Base, Part, Pending, Request, Run, Scope, TEMPORAL_SOURCES, TEMPORAL_TARGETS, concatenate,
};
use crate::map::{MapError, Outcome, RowRef};
use crate::parse::Location;

impl<'a> Run<'a> {
    /// Applies one row whose condition holds to a valued `datum`.
    pub(super) fn apply(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        datum: Datum<'a>,
        repetition: usize,
        base: &Base,
    ) -> Result<(), MapError> {
        let (instance, steps) = match &row.target {
            Ok(Target::Path { instance, steps }) => (instance.clone(), steps.as_slice()),
            Ok(Target::Value { instance }) => (instance.clone(), &[][..]),
            Err(error) => {
                self.outcomes.push(Outcome::UnsupportedTarget {
                    at: scope.at.clone(),
                    row: reference.clone(),
                    error: error.clone(),
                });
                return Ok(());
            }
        };
        let Some(base) = self.instance(base, instance.as_ref(), &scope.at, reference) else {
            return Ok(());
        };
        self.steps(scope, row, reference, datum, repetition, &base, steps)
    }

    /// Follows the steps of a target, through a `(Type)` reference when one
    /// ends them.
    #[expect(
        clippy::too_many_arguments,
        reason = "one row's full context; a struct for it would only rename the parameters"
    )]
    fn steps(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        datum: Datum<'a>,
        repetition: usize,
        base: &Base,
        steps: &[Step],
    ) -> Result<(), MapError> {
        match steps.iter().position(|step| step.reference.is_some()) {
            Some(position) if position.saturating_add(1) == steps.len() => {
                if row.assignment.is_none()
                    && self.sibling_reference(base, steps, repetition, &scope.at, reference)
                {
                    return Ok(());
                }
                let target = steps.last().and_then(|step| step.reference.clone());
                let inner = target
                    .as_ref()
                    .map(|target| target.path.clone())
                    .unwrap_or_default();
                if inner.is_empty() && row.assignment.is_none() && row.mapped_via.is_none() {
                    // NOTE: no specification governs this: our own design; a resource no data
                    // type map can fill is never created, so no empty resource enters the Bundle.
                    let source_type = row.source_type.clone().unwrap_or_default();
                    let resource = target
                        .as_ref()
                        .map(|target| target.resource.clone())
                        .unwrap_or_default();
                    if self
                        .corpus
                        .find("datatype", &source_type, &resource)
                        .is_err()
                    {
                        self.outcomes.push(Outcome::NoDatatypeMap {
                            at: scope.at.clone(),
                            row: reference.clone(),
                            source_type,
                            target_type: resource,
                        });
                        return Ok(());
                    }
                }
                let Some(created) = self.reference(base, steps, repetition, &scope.at, reference)
                else {
                    return Ok(());
                };
                if inner.is_empty() {
                    return self.value(scope, row, reference, datum, &created);
                }
                let resource = target
                    .as_ref()
                    .map(|target| target.resource.clone())
                    .unwrap_or_default();
                if let Some(map) = self.reference_root(row, &resource, &inner) {
                    let source_type = row.source_type.clone().unwrap_or_default();
                    self.datatype(scope, map, &source_type, datum, &created, &BTreeSet::new())?;
                    self.outcomes.push(Outcome::ReferenceRoot {
                        at: scope.at.clone(),
                        row: reference.clone(),
                        map: map.id.clone(),
                    });
                    return Ok(());
                }
                self.steps(scope, row, reference, datum, 0, &created, &inner)
            }
            Some(_) => {
                self.outcomes.push(Outcome::UnsupportedShape {
                    at: scope.at.clone(),
                    row: reference.clone(),
                });
                Ok(())
            }
            None => {
                let Some(slots) = self.slots(base, steps, repetition, &scope.at, reference) else {
                    return Ok(());
                };
                let leaf = Base {
                    resource: base.resource,
                    slots,
                };
                self.value(scope, row, reference, datum, &leaf)
            }
        }
    }

    /// The data type map from a `(Type)` row's source into the referenced
    /// `resource`, when the row's one step
    /// inside the resource is a `Reference` no data type map fills and every
    /// row of that map writes through a reference at that element.
    ///
    /// `OBX-25` into `performer(PractitionerRole.practitioner)` has no `XCN`
    /// map into `Reference`, and `datatype-xcn-to-practitionerrole` writes
    /// the XCN at `practitioner(Practitioner...)`, so the row runs that map at
    /// the `PractitionerRole`. No specification governs this: our own
    /// design, since the guide names no data type map per row.
    fn reference_root(&self, row: &Row, resource: &str, inner: &[Step]) -> Option<&'a Map> {
        let [step] = inner else {
            return None;
        };
        if step.reference.is_some() || row.assignment.is_some() || row.mapped_via.is_some() {
            return None;
        }
        let source_type = row.source_type.as_deref().filter(|name| !name.is_empty())?;
        // NOTE: no specification governs this: our own design; an element the table does not
        // resolve is counted by the ordinary path the row then takes.
        let resolved = resolve_path(resource, &[step.name.as_str()]).ok()?;
        let reference = matches!(
            resolved.location(),
            fhirconnect::tree::element::Location::Complex(schema) if schema.name == "Reference"
        );
        if !reference
            || !self
                .corpus
                .qualifying("datatype", source_type, "Reference")
                .is_empty()
        {
            return None;
        }
        let corpus = self.corpus;
        // NOTE: no specification governs this: our own design; no single map into the resource
        // leaves the row to the ordinary path, which counts it.
        let map = corpus.find("datatype", source_type, resource).ok()?;
        let through = map.rows.iter().all(|other| match &other.target {
            Ok(Target::Path { steps, .. }) => steps
                .first()
                .is_some_and(|first| first.name == step.name && first.reference.is_some()),
            _ => false,
        });
        (through && !map.rows.is_empty()).then_some(map)
    }

    /// Writes a row's `assignment` at `leaf`: a literal as given, a
    /// concatenation as the texts it joins, and any other form counted.
    fn assign(
        &mut self,
        scope: &Scope<'a>,
        assignment: &Assignment,
        resolved: &fhirconnect::tree::element::Resolved,
        leaf: &Base,
        reference: &RowRef,
    ) {
        let at = &scope.at;
        let row = || reference.clone();
        match assignment {
            Assignment::Literal(text) => {
                let value = match resolved.location() {
                    fhirconnect::tree::element::Location::Primitive(
                        fhir_types::schema::ValueKind::Boolean,
                    ) => match text.as_str() {
                        "true" => Value::Bool(true),
                        "false" => Value::Bool(false),
                        _ => Value::String(text.clone()),
                    },
                    _ => Value::String(text.clone()),
                };
                self.record(leaf, Pending::Ready(value), at, reference);
            }
            Assignment::Concat(parts) => match concatenate(scope, parts) {
                Ok(text) => self.record(leaf, Pending::Ready(Value::String(text)), at, reference),
                Err(operand) => self.outcomes.push(Outcome::UnevaluableAssignment {
                    at: at.clone(),
                    row: row(),
                    operand,
                }),
            },
            Assignment::Defective(text) => self.outcomes.push(Outcome::DefectiveAssignment {
                at: at.clone(),
                row: row(),
                text: text.clone(),
            }),
            Assignment::Unsupported(text) => self.outcomes.push(Outcome::UnsupportedAssignment {
                at: at.clone(),
                row: row(),
                text: text.clone(),
            }),
        }
    }

    /// Writes the value of `datum` at `leaf`: an assignment, a translation,
    /// a data type map or a converted primitive.
    pub(super) fn value(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        datum: Datum<'a>,
        leaf: &Base,
    ) -> Result<(), MapError> {
        let at = &scope.at;
        let Some(resolved) = self.resolve(leaf, at, reference) else {
            return Ok(());
        };
        if let Some(assignment) = &row.assignment {
            self.assign(scope, assignment, &resolved, leaf, reference);
            return Ok(());
        }
        let source_type = row.source_type.clone().unwrap_or_default();
        if let Some(mapped_via) = &row.mapped_via {
            let named = match mapped_via {
                MappedVia::Table(id) => {
                    self.corpus.get(id).filter(|map| map.kind == Kind::Datatype)
                }
                MappedVia::Unresolved(_) => None,
            };
            if let Some(map) = named {
                return self.datatype(scope, map, &source_type, datum, leaf, &BTreeSet::new());
            }
            self.table(leaf, &resolved, mapped_via, datum, at, reference);
            return Ok(());
        }
        let Some(target_type) = self.target_type(&resolved, at, reference) else {
            return Ok(());
        };
        let complex = matches!(
            resolved.location(),
            fhirconnect::tree::element::Location::Complex(_)
        );
        let temporal = TEMPORAL_SOURCES.contains(&source_type.as_str())
            && TEMPORAL_TARGETS.contains(&target_type.as_str());
        if complex {
            return self.complex(scope, row, reference, source_type, target_type, datum, leaf);
        }
        if !temporal
            && !source_type.is_empty()
            && let Ok(map) = self.corpus.find("datatype", &source_type, &target_type)
        {
            return self.datatype(scope, map, &source_type, datum, leaf, &BTreeSet::new());
        }
        if source_type == "HD" && target_type == "url" {
            self.endpoint(datum, leaf, at, reference);
            return Ok(());
        }
        let text = if source_type == "TS" {
            datum.child(1).text()
        } else {
            datum.text()
        };
        let Some(text) = text else {
            return Ok(());
        };
        if source_type != "TS" && datum.extra() {
            self.outcomes.push(Outcome::ComponentsDropped {
                at: at.clone(),
                row: reference.clone(),
            });
        }
        match convert::primitive(&target_type, text) {
            Ok(value) => self.record(leaf, Pending::Ready(value), at, reference),
            Err(error) => self.outcomes.push(Outcome::Unconvertible {
                at: at.clone(),
                row: reference.clone(),
                error,
            }),
        }
        Ok(())
    }

    /// Writes `datum` into the complex element `leaf` through the data type
    /// maps of [`Run::complex_maps`], counting `no-datatype-map` when there is
    /// none.
    #[expect(
        clippy::too_many_arguments,
        reason = "one row's full context; a struct for it would only rename the parameters"
    )]
    fn complex(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        source_type: String,
        target_type: String,
        datum: Datum<'a>,
        leaf: &Base,
    ) -> Result<(), MapError> {
        let maps = if source_type.is_empty() {
            Vec::new()
        } else {
            self.complex_maps(&source_type, &target_type, leaf)
        };
        if maps.is_empty() {
            if let Some(child) = self.value_child(row, leaf) {
                self.outcomes.push(Outcome::ValueChild {
                    at: scope.at.clone(),
                    row: reference.clone(),
                    target_type,
                });
                return self.value(scope, row, reference, datum, &child);
            }
            self.outcomes.push(Outcome::NoDatatypeMap {
                at: scope.at.clone(),
                row: reference.clone(),
                source_type,
                target_type,
            });
            return Ok(());
        }
        let chosen = self.select(scope, reference, maps, &source_type, datum, leaf)?;
        let (chosen, also) = self.halve(scope, row, reference, chosen, datum);
        match chosen.as_slice() {
            [] => Ok(()),
            [map] => self.datatype(scope, map, &source_type, datum, leaf, &also),
            several => self.datatypes(
                scope,
                row,
                reference,
                several,
                &source_type,
                datum,
                leaf,
                &also,
            ),
        }
    }

    /// The `value` child of the complex element `leaf` a data type map row
    /// writes at `$value`, when that child is a primitive.
    ///
    /// `datatype-st-to-identifier` maps `ST.1` to `$value` of an
    /// `Identifier`, which holds its text in `value`. No specification
    /// governs this: our own design, since the guide's notation writes the
    /// element itself (`mapping_guidelines.md` §\[n\] Notation).
    fn value_child(&self, row: &Row, leaf: &Base) -> Option<Base> {
        if !matches!(row.target, Ok(Target::Value { .. })) {
            return None;
        }
        let child = leaf.child("value", "");
        let resource_type = &self.resources.get(leaf.resource)?.type_name;
        let names: Vec<&str> = child.slots.iter().map(|slot| slot.name.as_str()).collect();
        // NOTE: no specification governs this: our own design; a type with no `value` child has
        // no place for the text, so the row stays `no-datatype-map`.
        let resolved = resolve_path(resource_type, &names).ok()?;
        matches!(
            resolved.location(),
            fhirconnect::tree::element::Location::Primitive(_)
        )
        .then_some(child)
    }

    /// Records the translation of a table value, written as the code, the
    /// Coding or the `CodeableConcept` the target element holds.
    ///
    /// No specification governs where a translated concept lands in a
    /// complex target: our own design writes a Coding's `system`, `code` and
    /// `display`, and a `CodeableConcept`'s first `coding`.
    fn table(
        &mut self,
        leaf: &Base,
        resolved: &fhirconnect::tree::element::Resolved,
        mapped_via: &MappedVia,
        datum: Datum<'a>,
        at: &Location,
        reference: &RowRef,
    ) {
        let found = match mapped_via {
            MappedVia::Table(id) => self.corpus.get(id).filter(|map| map.kind == Kind::Table),
            MappedVia::Unresolved(_) => None,
        };
        let Some(map) = found else {
            let text = match mapped_via {
                MappedVia::Table(id) => format!("ConceptMap/{id}"),
                MappedVia::Unresolved(text) => text.clone(),
            };
            self.outcomes.push(Outcome::UnresolvedTable {
                at: at.clone(),
                row: reference.clone(),
                mapped_via: text,
            });
            return;
        };
        let Some(code) = datum.text() else {
            return;
        };
        let request = self.requests.len();
        self.requests.push(Request {
            code: String::from(code),
            url: map.url.clone(),
            groups: map.groups.clone(),
            at: at.clone(),
            row: reference.clone(),
            answer: None,
        });
        let pending = |part| Pending::Translation { request, part };
        match resolved.location() {
            fhirconnect::tree::element::Location::Complex(schema) if schema.name == "Coding" => {
                for (name, part) in [
                    ("system", Part::System),
                    ("code", Part::Code),
                    ("display", Part::Display),
                ] {
                    self.record(&leaf.child(name, ""), pending(part), at, reference);
                }
            }
            fhirconnect::tree::element::Location::Complex(schema)
                if schema.name == "CodeableConcept" =>
            {
                let coding = leaf.child("coding", "");
                for (name, part) in [
                    ("system", Part::System),
                    ("code", Part::Code),
                    ("display", Part::Display),
                ] {
                    self.record(&coding.child(name, ""), pending(part), at, reference);
                }
            }
            _ => self.record(leaf, pending(Part::Code), at, reference),
        }
    }
}

#[cfg(test)]
mod tests {
    use fhir_types::codec::Value;

    use crate::map::Outcome;
    use crate::map::run::Run;
    use crate::map::run::tests::{
        GLUCOSE, corpus, ei_refusals, message, shipped, specimen, text, values_at, writes_of,
    };

    // NOTE: `segment-obx-to-observation` OBX-25 into `performer[2](PractitionerRole[1].practitioner)`,
    // and `datatype-xcn-to-practitionerrole` writes each XCN row at `practitioner(Practitioner...)`.
    #[test]
    fn a_responsible_observer_gives_a_practitioner_role_and_its_practitioner() {
        let corpus = corpus();
        let obx = format!("{GLUCOSE}{}PRV-1^Doe^Jane", "|".repeat(14));
        let parsed = specimen("ORC|RE|PLC-1|FIL-1", &obx, "SPM|1|SPC-P");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "Practitioner", "identifier.value"),
            vec![text("PRV-1")],
            "{:?}",
            writes_of(&run, "Practitioner")
        );
        assert_eq!(
            values_at(&run, "PractitionerRole", "code.coding.code"),
            vec![text("MDIR")]
        );
        assert_eq!(
            values_at(&run, "PractitionerRole", "practitioner.reference").len(),
            1,
            "{:?}",
            writes_of(&run, "PractitionerRole")
        );
        assert!(
            !values_at(&run, "Observation", "performer.reference").is_empty(),
            "{:?}",
            writes_of(&run, "Observation")
        );
        let roots: Vec<(&str, &str)> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::ReferenceRoot { row, map, .. } => {
                    Some((row.source.as_str(), map.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(roots, vec![("OBX-25", "datatype-xcn-to-practitionerrole")]);
        assert!(
            run.outcomes.iter().all(|outcome| !matches!(
                outcome,
                Outcome::NoDatatypeMap { row, .. } if row.source == "OBX-25"
            )),
            "{:?}",
            run.outcomes
        );
    }

    // NOTE: `segment-txa-to-documentreference` TXA-16 into `identifier[1]`, and
    // `datatype-st-to-identifier` writes `ST.1` at `$value`, which lands in `value`.
    #[test]
    fn a_unique_document_file_name_gives_an_identifier_value() {
        let corpus = corpus();
        let parsed = message(&[
            "MSH|^~\\&|DOCS|NORTHHOSP|EHR|SOUTHCLINIC|20260925100000+0200||MDM^T02^MDM_T02|MSG00032|P|2.5.1",
            "EVN||20260925095900+0200",
            "PID|1||PAT-0032^^^NORTHHOSP^MR||Doe^Sam^^^^^L||19751111|U",
            "PV1|1|O",
            "TXA|1|CN|TX|20260925095000+0200||||||||DOC-0032||||FILE-0032|AU",
        ]);
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DocumentReference", "identifier.value"),
            vec![text("FILE-0032")],
            "{:?}",
            writes_of(&run, "DocumentReference")
        );
        assert!(
            run.outcomes.iter().any(|outcome| matches!(
                outcome,
                Outcome::ValueChild { row, target_type, .. }
                    if row.map == "datatype-st-to-identifier" && target_type == "Identifier"
            )),
            "{:?}",
            run.outcomes
        );
        assert_eq!(ei_refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: HL7 v2.3 types OBR-4 `CE`, so the order runs `datatype-ce-to-codeableconcept`, whose
    // CE.1 row the supplement runs with no Narrative-Condition.
    #[test]
    fn ce_one_is_the_code_of_a_legacy_order() {
        let parsed = message(&[
            "MSH|^~\\&|NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC|20260925143000+0200||ORM^O01|MSG00044|P|2.3",
            "PID|1||PAT-0044^^^NORTHHOSP^MR||Doe^Sam||19800101|M",
            "ORC|NW|PLC-44",
            "OBR|1|PLC-44||2345-7^Glucose^LN",
        ]);
        let guide = corpus();
        let mut run = Run::new(&guide, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "ServiceRequest", "code.coding.code"),
            Vec::<Value>::new()
        );
        let supplemented = shipped();
        let mut run = Run::new(&supplemented, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "ServiceRequest", "code.coding.code"),
            vec![text("2345-7")],
            "{:?}",
            writes_of(&run, "ServiceRequest")
        );
        assert!(run.outcomes.iter().any(|outcome| matches!(
            outcome,
            Outcome::Supplemented { map, overrides: true, .. }
                if map == "datatype-ce-to-codeableconcept"
        )));
    }
}
