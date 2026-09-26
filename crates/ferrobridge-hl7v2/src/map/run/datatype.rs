// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data type maps a field or component runs, and the rollback of a
//! complex target whose maps conflict.

use std::collections::{BTreeMap, BTreeSet};

use crate::map::condition::{self, Operand};
use crate::map::corpus::{Condition, Map, Row};
use crate::map::notation::Target;
use crate::map::run::datum::Datum;
use crate::map::run::segment::row_ref;
use crate::map::run::segment::split_source;
use crate::map::run::write::resolve_path;
use crate::map::run::{Base, Covered, Mark, Pending, Run, Scope, Slot, Written};
use crate::map::{MapError, Outcome, RowRef};

impl<'a> Run<'a> {
    /// Runs a data type map over `datum` into `base`, counting each valued
    /// component neither its rows nor `also` name.
    pub(super) fn datatype(
        &mut self,
        scope: &Scope<'a>,
        map: &'a Map,
        datatype: &str,
        datum: Datum<'a>,
        base: &Base,
        also: &BTreeSet<usize>,
    ) -> Result<(), MapError> {
        let mut covered =
            self.datatype_rows(scope, map, datatype, datum, base, &BTreeSet::new())?;
        covered.named.extend(also);
        self.unmapped_components(scope, &map.id, datum, &covered.named);
        Ok(())
    }

    /// Runs every data type map of `maps` over `datum` into the complex
    /// element `leaf`, each writing its own children of it.
    ///
    /// A child another row of the same source targets directly is that row's
    /// ([`Run::claimed`]), so no map writes it. When two maps write one child
    /// for this value, the row is refused whole: everything the maps recorded
    /// is rolled back and each such child counted as
    /// [`Outcome::DatatypeConflict`]. No specification governs this: our own
    /// design, since the guide names no data type map per row.
    #[expect(
        clippy::too_many_arguments,
        reason = "one row's full context; a struct for it would only rename the parameters"
    )]
    pub(super) fn datatypes(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        maps: &[&'a Map],
        datatype: &str,
        datum: Datum<'a>,
        leaf: &Base,
        also: &BTreeSet<usize>,
    ) -> Result<(), MapError> {
        let resource_type = self
            .resources
            .get(leaf.resource)
            .map(|resource| resource.type_name.clone())
            .unwrap_or_default();
        let claimed = self.claimed(row, reference, &resource_type);
        let mark = self.mark();
        let mut named = also.clone();
        let mut by_child: BTreeMap<String, Vec<Written>> = BTreeMap::new();
        for map in maps {
            let start = self
                .resources
                .get(leaf.resource)
                .map_or(0, |resource| resource.writes.len());
            named.extend(
                self.datatype_rows(scope, map, datatype, datum, leaf, &claimed)?
                    .named,
            );
            let recorded = self
                .resources
                .get(leaf.resource)
                .and_then(|resource| resource.writes.get(start..))
                .unwrap_or_default();
            let mut children: BTreeMap<String, Written> = BTreeMap::new();
            for (offset, write) in recorded.iter().enumerate() {
                let Some(child) = child_of(&leaf.slots, &write.slots) else {
                    continue;
                };
                let entry = children.entry(child).or_insert_with(|| Written {
                    map: map.id.clone(),
                    indices: Vec::new(),
                    values: Vec::new(),
                });
                entry.indices.push(start.saturating_add(offset));
                entry.values.push((
                    write
                        .slots
                        .get(leaf.slots.len()..)
                        .unwrap_or_default()
                        .to_vec(),
                    match &write.value {
                        Pending::Ready(value) => Some(value.clone()),
                        Pending::Translation { .. } | Pending::Sibling { .. } => None,
                    },
                ));
            }
            for (child, written) in children {
                by_child.entry(child).or_default().push(written);
            }
        }
        by_child.retain(|_, written| written.len() > 1);
        // NOTE: no specification governs this: our own design; maps that write one child with the
        // same values agree, so the first map's writes stand and the others' are dropped.
        let mut duplicates = Vec::new();
        by_child.retain(|_, written| {
            let agree = written.split_first().is_some_and(|(first, rest)| {
                first.values.iter().all(|(_, value)| value.is_some())
                    && rest.iter().all(|other| other.values == first.values)
            });
            if agree {
                duplicates.extend(
                    written
                        .iter()
                        .skip(1)
                        .flat_map(|other| other.indices.clone()),
                );
            }
            !agree
        });
        if by_child.is_empty() {
            duplicates.sort_unstable();
            if let Some(resource) = self.resources.get_mut(leaf.resource) {
                for index in duplicates.into_iter().rev() {
                    if index < resource.writes.len() {
                        resource.writes.remove(index);
                    }
                }
            }
            let ids: Vec<&str> = maps.iter().map(|map| map.id.as_str()).collect();
            self.unmapped_components(scope, &ids.join(" "), datum, &named);
            return Ok(());
        }
        self.rollback(&mark);
        for (element, written) in by_child {
            let maps = written.into_iter().map(|written| written.map).collect();
            self.outcomes.push(Outcome::DatatypeConflict {
                at: scope.at.clone(),
                row: reference.clone(),
                element,
                maps,
            });
        }
        Ok(())
    }

    /// The lengths of what the run has recorded so far, for [`Run::rollback`].
    pub(super) fn mark(&self) -> Mark {
        Mark {
            writes: self
                .resources
                .iter()
                .map(|resource| resource.writes.len())
                .collect(),
            requests: self.requests.len(),
            outcomes: self.outcomes.len(),
        }
    }

    /// Drops every resource, write, translation request and outcome recorded
    /// since `mark`.
    pub(super) fn rollback(&mut self, mark: &Mark) {
        self.resources.truncate(mark.writes.len());
        for (resource, length) in self.resources.iter_mut().zip(&mark.writes) {
            resource.writes.truncate(*length);
        }
        self.requests.truncate(mark.requests);
        self.outcomes.truncate(mark.outcomes);
    }

    /// The children of a row's complex target that another row of its map,
    /// from the same source, targets directly.
    ///
    /// A row that maps only the absence of that source
    /// ([`condition::requires_absent`]) never meets a valued one, so it claims
    /// nothing, and neither does a row the bridge's endpoint fallback fills
    /// ([`Run::falls_back`]), since a row of the guide wins over it.
    fn claimed(&self, row: &Row, reference: &RowRef, resource_type: &str) -> BTreeSet<String> {
        let Ok(Target::Path { steps: own, .. }) = &row.target else {
            return BTreeSet::new();
        };
        let Some(map) = self.corpus.get(&reference.map) else {
            return BTreeSet::new();
        };
        let operand = source_operand(&row.source);
        map.rows
            .iter()
            .filter(|other| other.source == row.source && other.target_code != row.target_code)
            .filter(|other| {
                !matches!(&other.condition, Condition::Computable(expr)
                    if condition::requires_absent(expr, &operand))
            })
            .filter(|other| !self.falls_back(resource_type, other))
            .filter_map(|other| match &other.target {
                Ok(Target::Path { steps, .. }) => {
                    let under = steps.len() > own.len()
                        && steps
                            .iter()
                            .zip(own)
                            .all(|(step, mine)| step.name == mine.name);
                    under
                        .then(|| steps.get(own.len()).map(|step| step.name.clone()))
                        .flatten()
                }
                _ => None,
            })
            .collect()
    }

    /// Whether `row` writes an `HD` into a `url` of `resource_type` through
    /// [`Run::endpoint`]: no assignment, no table and no data type map.
    fn falls_back(&self, resource_type: &str, row: &Row) -> bool {
        let Ok(Target::Path { steps, .. }) = &row.target else {
            return false;
        };
        let names: Vec<&str> = steps.iter().map(|step| step.name.as_str()).collect();
        row.assignment.is_none()
            && row.mapped_via.is_none()
            && row.source_type.as_deref() == Some("HD")
            && self.corpus.find("datatype", "HD", "url").is_err()
            && resolve_path(resource_type, &names)
                .is_ok_and(|resolved| resolved.type_code() == Some("url"))
    }

    /// Counts each valued component of `datum` that no row of the data type
    /// maps named.
    fn unmapped_components(
        &mut self,
        scope: &Scope<'a>,
        map: &str,
        datum: Datum<'a>,
        named: &BTreeSet<usize>,
    ) {
        for component in datum.valued_parts() {
            if !named.contains(&component) {
                self.outcomes.push(Outcome::UnmappedComponent {
                    at: scope.at.clone(),
                    map: String::from(map),
                    component,
                });
            }
        }
    }

    /// Runs the rows of a data type map over `datum` into `base`, skipping
    /// each row whose target starts at a child in `claimed`, and returns the
    /// components the rows name and those whose rows wrote.
    pub(super) fn datatype_rows(
        &mut self,
        scope: &Scope<'a>,
        map: &'a Map,
        datatype: &str,
        datum: Datum<'a>,
        base: &Base,
        claimed: &BTreeSet<String>,
    ) -> Result<Covered, MapError> {
        self.supplemented(map, &scope.at);
        let mut inner = scope.clone();
        inner.datatype = Some((String::from(datatype), datum));
        inner.repetition = None;
        let mut covered = Covered::default();
        // The component of the row before, with the write count it started at.
        let mut started: Option<(usize, usize)> = None;
        for row in &map.rows {
            covered.settle(started.take(), self.pushed);
            let reference = row_ref(map, row);
            let (id, path) = split_source(&row.source, '.');
            if id != datatype {
                self.outcomes.push(Outcome::UnsupportedShape {
                    at: scope.at.clone(),
                    row: reference,
                });
                continue;
            }
            if let Some(first) = path.first() {
                covered.named.insert(*first);
                started = Some((*first, self.pushed));
            }
            let owned = match &row.target {
                Ok(Target::Path { steps, .. }) => steps
                    .first()
                    .is_some_and(|step| claimed.contains(&step.name)),
                _ => false,
            };
            if owned {
                continue;
            }
            let part = datum.at(&path);
            if !part.valued() {
                let own = Operand::Component {
                    datatype: String::from(id),
                    path,
                };
                self.absence(&inner, row, &reference, &own, base)?;
                continue;
            }
            if !self.gate(&inner, row, &reference)? {
                continue;
            }
            self.apply(&inner, row, &reference, part, 0, base)?;
        }
        covered.settle(started, self.pushed);
        Ok(covered)
    }
}

/// The name of the child of `prefix` a write at `slots` lies under, when it
/// lies under `prefix` at all.
fn child_of(prefix: &[Slot], slots: &[Slot]) -> Option<String> {
    let under = slots.len() > prefix.len()
        && prefix
            .iter()
            .zip(slots)
            .all(|(outer, slot)| outer.name == slot.name);
    under
        .then(|| slots.get(prefix.len()).map(|slot| slot.name.clone()))
        .flatten()
}

/// The operand a row's source code reads: a field for `MSH-24`, a component
/// for `HD.1`.
fn source_operand(source: &str) -> Operand {
    if source.contains('-') {
        let (segment, path) = split_source(source, '-');
        Operand::Field {
            segment: String::from(segment),
            path,
        }
    } else {
        let (datatype, path) = split_source(source, '.');
        Operand::Component {
            datatype: String::from(datatype),
            path,
        }
    }
}

#[cfg(test)]
mod tests {
    use fhir_types::codec::{Json, Path, Value};

    use crate::map::Outcome;
    use crate::map::run::Run;
    use crate::map::run::tests::{corpus, datatype_map, header_writes, parsed_with, supplemented};

    /// MSH-24 alone valued, as `NORTHNET^1.2.3.9^ISO`.
    const NETWORK_ONLY: (&str, &str) = (
        "|NORTHLAB|EHR|SOUTHCLINIC",
        "||||||||||||NORTHNET^1.2.3.9^ISO",
    );

    fn header_write<'w>(writes: &'w [(String, Value)], path: &str) -> Option<&'w Value> {
        writes
            .iter()
            .find(|(written, _)| written == path)
            .map(|(_, value)| value)
    }

    // NOTE: `ConceptMap-datatype-hd-endpoint-to-messageheader-source` and `-hd-name-`: both
    // name `MessageHeader.source` as their target, so a valued MSH-24 runs both into it.
    #[test]
    fn a_row_into_a_complex_target_runs_every_map_named_for_its_element_path() {
        let corpus = corpus();
        let parsed = parsed_with(NETWORK_ONLY.0, NETWORK_ONLY.1);
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let writes = header_writes(&run);
        assert_eq!(
            header_write(&writes, "source.name"),
            Some(&Value::String(String::from("NORTHNET"))),
            "{writes:?}"
        );
        assert_eq!(
            header_write(&writes, "source.software"),
            Some(&Value::String(String::from("1.2.3.9"))),
            "{writes:?}"
        );
        assert_eq!(
            header_write(&writes, "source.endpoint"),
            Some(&Value::String(String::from("urn:oid:1.2.3.9"))),
            "the endpoint map's ISO row runs: {writes:?}"
        );
        assert!(
            run.outcomes.iter().all(|outcome| !matches!(
                outcome,
                Outcome::NoDatatypeMap { .. } | Outcome::DatatypeConflict { .. }
            )),
            "{:?}",
            run.outcomes
        );
    }

    // NOTE: HL7 R4 Bundle bdl-12: with an endpoint map whose rows run, MSH-24 alone gives
    // `source.endpoint` and `source.name` from two maps, and the message Bundle keeps its header.
    #[test]
    fn two_maps_into_distinct_children_complete_the_message_header() {
        let url = "http://hl7.org/fhir/uv/v2mappings/ConceptMap/datatype-hd-endpoint-to-messageheader-source";
        let endpoint = datatype_map(
            "datatype-hd-endpoint-to-messageheader-source",
            url,
            &[("HD.2", "endpoint", "url")],
        );
        let corpus = supplemented(&[endpoint]);
        let parsed = parsed_with(NETWORK_ONLY.0, NETWORK_ONLY.1);
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let (bundle, outcomes) = run.finish().expect("the Bundle keeps its MessageHeader");
        let object = bundle.as_object().expect("a Bundle object");
        fhir_types::r4::bundle::Bundle::from_json(object, &mut Path::root("Bundle"))
            .expect("the Bundle decodes as R4");
        let header = bundle
            .get("entry")
            .and_then(Value::as_array)
            .and_then(<[Value]>::first)
            .and_then(|entry| entry.get("resource"))
            .expect("a first entry");
        assert_eq!(
            header.get("resourceType").and_then(Value::as_str),
            Some("MessageHeader")
        );
        let source = header.get("source").expect("a source");
        assert_eq!(
            source.get("endpoint").and_then(Value::as_str),
            Some("1.2.3.9"),
            "{source:?}"
        );
        assert_eq!(
            source.get("name").and_then(Value::as_str),
            Some("NORTHNET"),
            "{source:?}"
        );
        assert!(
            outcomes
                .iter()
                .all(|outcome| !matches!(outcome, Outcome::DatatypeConflict { .. })),
            "{outcomes:?}"
        );
    }

    // NOTE: no specification governs this: our own design; two maps writing one child of the
    // target with different values refuse the row, counted once per child, and neither value lands.
    #[test]
    fn two_maps_into_one_child_refuse_the_row_as_a_counted_conflict() {
        let conflicting = datatype_map(
            "datatype-hd-label-to-messageheader-source",
            "http://example.org/fhir/ConceptMap/datatype-hd-label-to-messageheader-source",
            &[("HD.2", "name", "string")],
        );
        let corpus = supplemented(&[conflicting]);
        let parsed = parsed_with(NETWORK_ONLY.0, NETWORK_ONLY.1);
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let writes = header_writes(&run);
        assert!(
            writes.iter().all(|(path, _)| !path.starts_with("source")),
            "{writes:?}"
        );
        let conflicts: Vec<(&str, &[String])> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::DatatypeConflict { element, maps, .. } => {
                    Some((element.as_str(), maps.as_slice()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            conflicts,
            vec![(
                "name",
                &[
                    String::from("datatype-hd-label-to-messageheader-source"),
                    String::from("datatype-hd-name-to-messageheader-source"),
                ][..]
            )]
        );
    }

    // NOTE: no specification governs this: our own design; maps that write one child with the
    // same value agree, so the value is written once and no conflict is counted.
    #[test]
    fn two_maps_writing_one_child_with_one_value_agree() {
        let agreeing = datatype_map(
            "datatype-hd-label-to-messageheader-source",
            "http://example.org/fhir/ConceptMap/datatype-hd-label-to-messageheader-source",
            &[("HD.1", "name", "string")],
        );
        let corpus = supplemented(&[agreeing]);
        let parsed = parsed_with(NETWORK_ONLY.0, NETWORK_ONLY.1);
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let writes = header_writes(&run);
        let names: Vec<&Value> = writes
            .iter()
            .filter(|(path, _)| path == "source.name")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(
            names,
            vec![&Value::String(String::from("NORTHNET"))],
            "{writes:?}"
        );
        assert!(
            run.outcomes
                .iter()
                .all(|outcome| !matches!(outcome, Outcome::DatatypeConflict { .. })),
            "{:?}",
            run.outcomes
        );
    }

    // NOTE: `datatype-hd-endpoint-to-messageheader-source` writes `name` as `HD.1+" - "+HD.3+":"+HD.2`
    // for another HD.3 and `datatype-hd-name-to-messageheader-source` writes HD.1, so they conflict.
    #[test]
    fn a_namespace_id_beside_another_universal_id_type_is_a_conflict_of_the_guides_maps() {
        let corpus = corpus();
        let parsed = parsed_with(NETWORK_ONLY.0, "||||||||||||NORTHNET^LAB-7^L");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let conflicts: Vec<(&str, &[String])> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::DatatypeConflict { element, maps, .. } => {
                    Some((element.as_str(), maps.as_slice()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            conflicts,
            vec![(
                "name",
                &[
                    String::from("datatype-hd-endpoint-to-messageheader-source"),
                    String::from("datatype-hd-name-to-messageheader-source"),
                ][..]
            )],
            "{:?}",
            run.outcomes
        );
    }
}
