// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One run of the interpreter over one parsed message.
//!
//! The run walks the message synchronously and records every value as a
//! write against a resource: the path with an instance key per step, and the
//! value or the translation it waits for. The translations are then asked of
//! the terminology server in the order the walk met them, and the writes are
//! applied through `fhirconnect::tree::write`, which is where each instance
//! key receives its array index: in the order a value first reaches it.

use std::collections::{BTreeMap, BTreeSet};

use fhir_types::codec::{Json, Object, Value};
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::tree::Occurrence;
use fhirconnect::tree::element::{Move, resolve};
use fhirconnect::tree::path::FhirPath;
use fhirconnect::tree::write::write;
use hl7v2_types::model::Max;
use sha2::{Digest, Sha256};

use crate::map::condition::{self, Operand, Probe, Unevaluable};
use crate::map::convert;
use crate::map::corpus::{Assignment, Condition, Corpus, Kind, Map, MappedVia, Row, TableGroup};
use crate::map::notation::{Label, Step, Target};
use crate::map::{ElementError, MapError, Outcome, RowRef};
use crate::parse::{Component, Item, Location, Parsed, Repetition, Segment};

/// The v2 date and time types the mapping guidelines convert to FHIR date and
/// time types (`mapping_guidelines.md` §Data Type Spreadsheet).
const TEMPORAL_SOURCES: [&str; 4] = ["DTM", "DT", "TS", "TM"];

/// The FHIR types those conversions reach.
const TEMPORAL_TARGETS: [&str; 4] = ["date", "dateTime", "instant", "time"];

/// A v2 value at some depth of a field.
#[derive(Debug, Clone, Copy)]
enum Datum<'m> {
    /// A whole segment.
    Segment,
    /// One repetition of a field.
    Repetition(&'m Repetition),
    /// One component.
    Component(&'m Component),
    /// One subcomponent's text.
    Text(&'m str),
    /// Nothing at that position.
    Absent,
}

impl<'m> Datum<'m> {
    /// The part at `position`, from 1, one level down.
    fn child(self, position: usize) -> Self {
        match self {
            Self::Repetition(repetition) => repetition
                .component(position)
                .map_or(Self::Absent, Self::Component),
            Self::Component(component) => component
                .subcomponents()
                .get(position.saturating_sub(1))
                .map_or(Self::Absent, |text| Self::Text(text.as_str())),
            Self::Text(text) if position == 1 => Self::Text(text),
            Self::Segment | Self::Text(_) | Self::Absent => Self::Absent,
        }
    }

    /// The part a component path names.
    fn at(self, path: &[usize]) -> Self {
        path.iter()
            .fold(self, |datum, position| datum.child(*position))
    }

    /// The first leaf's text, when it is valued.
    fn text(self) -> Option<&'m str> {
        match self {
            Self::Repetition(repetition) => repetition.text(),
            Self::Component(component) => component.text(),
            Self::Text(text) => Some(text).filter(|text| crate::parse::valued(text)),
            Self::Segment | Self::Absent => None,
        }
    }

    /// Whether any leaf holds a value.
    fn valued(self) -> bool {
        match self {
            Self::Segment => true,
            Self::Repetition(repetition) => repetition.is_valued(),
            Self::Component(component) => component.is_valued(),
            Self::Text(text) => crate::parse::valued(text),
            Self::Absent => false,
        }
    }

    /// Whether a leaf after the first holds a value.
    fn extra(self) -> bool {
        match self {
            Self::Repetition(repetition) => {
                repetition
                    .components()
                    .iter()
                    .skip(1)
                    .any(Component::is_valued)
                    || repetition
                        .component(1)
                        .is_some_and(|first| Datum::Component(first).extra())
            }
            Self::Component(component) => component
                .subcomponents()
                .iter()
                .skip(1)
                .any(|text| crate::parse::valued(text)),
            Self::Segment | Self::Text(_) | Self::Absent => false,
        }
    }

    /// The positions of the valued parts one level down.
    fn valued_parts(self) -> Vec<usize> {
        let count = match self {
            Self::Repetition(repetition) => repetition.components().len(),
            Self::Component(component) => component.subcomponents().len(),
            Self::Segment | Self::Text(_) | Self::Absent => 0,
        };
        (1..=count)
            .filter(|position| self.child(*position).valued())
            .collect()
    }
}

/// Where the rows of one map are evaluated.
#[derive(Debug, Clone)]
struct Scope<'p> {
    parsed: &'p Parsed,
    index: usize,
    chain: Vec<&'p [Item]>,
    at: Location,
    repetition: Option<(usize, usize)>,
    datatype: Option<(String, Datum<'p>)>,
}

impl<'p> Scope<'p> {
    /// The segment the scope maps.
    fn segment(&self) -> Option<&'p Segment> {
        self.parsed.message().segments().get(self.index)
    }

    /// The nearest segment with `id`: the one mapped, else the first in the
    /// innermost enclosing group instance that holds one.
    fn find(&self, id: &str) -> Option<&'p Segment> {
        let current = self.segment()?;
        if current.id() == id {
            return Some(current);
        }
        let segments = self.parsed.message().segments();
        self.chain
            .iter()
            .find_map(|items| first_placed(items, id, segments))
    }

    /// Reads an operand of a condition.
    fn probe(&self, operand: &Operand) -> Option<Probe> {
        match operand {
            Operand::Segment(id) => {
                hl7v2_types::segment::find(id)?;
                Some(Probe {
                    valued: self.find(id).is_some(),
                    text: None,
                    count: u64::from(self.find(id).is_some()),
                })
            }
            Operand::Field { segment, path } => {
                hl7v2_types::segment::find(segment)?;
                let Some(found) = self.find(segment) else {
                    return Some(Probe::default());
                };
                let (&position, rest) = path.split_first()?;
                let Some(field) = found.field(position) else {
                    return Some(Probe::default());
                };
                let count = field
                    .repetitions()
                    .iter()
                    .filter(|repetition| repetition.is_valued())
                    .count();
                let mapped = self
                    .segment()
                    .is_some_and(|current| core::ptr::eq(current, found));
                let repetition = match self.repetition {
                    Some((row_field, repetition)) if mapped && row_field == position => repetition,
                    _ => 0,
                };
                let datum = field
                    .repetitions()
                    .get(repetition)
                    .map_or(Datum::Absent, Datum::Repetition)
                    .at(rest);
                Some(Probe {
                    valued: datum.valued(),
                    text: datum.text().map(String::from),
                    count: u64::try_from(count).unwrap_or(u64::MAX),
                })
            }
            Operand::Component { datatype, path } => {
                let (current, datum) = self.datatype.as_ref()?;
                if current != datatype {
                    return None;
                }
                let datum = datum.at(path);
                Some(Probe {
                    valued: datum.valued(),
                    text: datum.text().map(String::from),
                    count: u64::from(datum.valued()),
                })
            }
        }
    }
}

/// The first placed segment with `id` among `items`, searching group
/// instances depth first.
fn first_placed<'p>(items: &'p [Item], id: &str, segments: &'p [Segment]) -> Option<&'p Segment> {
    items.iter().find_map(|item| match item {
        Item::Segment(placed) => segments
            .get(placed.index)
            .filter(|segment| segment.id() == id),
        Item::Group(instance) => first_placed(&instance.items, id, segments),
    })
}

/// One step of a recorded write: the element name and its instance key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Slot {
    name: String,
    key: String,
}

/// Where a map's rows write: a resource and the steps into it.
#[derive(Debug, Clone)]
struct Base {
    resource: usize,
    slots: Vec<Slot>,
}

/// Which part of a translated concept a write takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Code,
    System,
    Display,
}

/// The value a write carries.
#[derive(Debug, Clone)]
enum Pending {
    Ready(Value),
    Translation { request: usize, part: Part },
}

/// One recorded write.
#[derive(Debug, Clone)]
struct Write {
    slots: Vec<Slot>,
    value: Pending,
    at: Location,
    row: RowRef,
}

/// How a resource came to exist.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Identity {
    /// The Bundle itself.
    Envelope,
    /// A resource the message map names, with the occurrence key of the
    /// segment that created it.
    Message {
        label: Option<Label>,
        key: Vec<usize>,
    },
    /// A resource a `(Type)` reference created from an element of another.
    Created {
        parent: usize,
        slots: Vec<(String, String)>,
        label: Option<Label>,
    },
}

/// One resource the run builds.
#[derive(Debug, Clone)]
struct Resource {
    type_name: String,
    identity: Identity,
    full_url: String,
    writes: Vec<Write>,
}

/// One translation the walk asks for.
#[derive(Debug, Clone)]
struct Request {
    code: String,
    url: String,
    groups: Vec<TableGroup>,
    at: Location,
    row: RowRef,
    answer: Option<ferrobridge_term::concept::Concept>,
}

/// A run in progress.
#[derive(Debug)]
pub(super) struct Run<'a> {
    corpus: &'a Corpus,
    parsed: &'a Parsed,
    resources: Vec<Resource>,
    requests: Vec<Request>,
    outcomes: Vec<Outcome>,
}

impl<'a> Run<'a> {
    /// Starts a run with the envelope Bundle as its first resource.
    pub(super) fn new(corpus: &'a Corpus, parsed: &'a Parsed) -> Self {
        let mut run = Self {
            corpus,
            parsed,
            resources: Vec::new(),
            requests: Vec::new(),
            outcomes: parsed
                .unplaced()
                .iter()
                .cloned()
                .map(Outcome::Parse)
                .collect(),
        };
        run.create("Bundle", Identity::Envelope);
        run
    }

    /// Walks the message through its message map.
    pub(super) fn message(&mut self) -> Result<(), MapError> {
        let structure = self.parsed.structure_name();
        let map = self
            .corpus
            .message_map(structure)
            .ok_or_else(|| MapError::NoMessageMap {
                structure: String::from(structure),
            })?;
        let mut visits = Vec::new();
        collect(
            self.parsed.items(),
            structure,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            self.parsed.structure().nodes,
            &mut visits,
        );
        visits.sort_by_key(|visit| visit.index);
        for visit in visits {
            self.visit(map, &visit)?;
        }
        Ok(())
    }

    /// Maps one placed segment through the message map rows naming it.
    fn visit(&mut self, map: &'a Map, visit: &Visit<'a>) -> Result<(), MapError> {
        let at = self.parsed.location(visit.index);
        let rows: Vec<&Row> = map
            .rows
            .iter()
            .filter(|row| visit.matches(&row.source))
            .collect();
        if rows.is_empty() {
            self.outcomes.push(Outcome::UnmappedSegment { at });
            return Ok(());
        }
        let scope = Scope {
            parsed: self.parsed,
            index: visit.index,
            chain: visit.chain.clone(),
            at: at.clone(),
            repetition: None,
            datatype: None,
        };
        let Some(segment) = scope.segment() else {
            return Ok(());
        };
        let mut named = BTreeSet::new();
        let mut ran = false;
        for row in rows {
            let reference = row_ref(map, row);
            if !self.gate(&scope, row, &reference)? {
                continue;
            }
            let Ok(Target::Path { steps, .. }) = &row.target else {
                if let Err(error) = &row.target {
                    self.outcomes.push(Outcome::UnsupportedTarget {
                        at: at.clone(),
                        row: reference,
                        error: error.clone(),
                    });
                }
                continue;
            };
            let Some((head, rest)) = steps.split_first() else {
                continue;
            };
            let target_type = rest
                .last()
                .and_then(|step| step.reference.as_ref())
                .map_or(head.name.as_str(), |reference| reference.resource.as_str());
            let segment_map = match self.corpus.find("segment", segment.id(), target_type) {
                Ok(found) => found,
                Err(candidates) => {
                    self.outcomes.push(Outcome::NoSegmentMap {
                        at: at.clone(),
                        resource: String::from(target_type),
                        candidates,
                    });
                    continue;
                }
            };
            let resource = if head.name == "Bundle" {
                0
            } else {
                self.message_resource(&head.name, head.label.clone(), &visit.key)
            };
            let base = Base {
                resource,
                slots: Vec::new(),
            };
            let base = match rest.iter().position(|step| step.reference.is_some()) {
                Some(position) if position.saturating_add(1) == rest.len() => {
                    let referencing = rest.get(..=position).unwrap_or_default();
                    let Some(created) = self.reference(&base, referencing, 0, &at, &reference)
                    else {
                        continue;
                    };
                    created
                }
                Some(_) => {
                    self.outcomes.push(Outcome::UnsupportedShape {
                        at: at.clone(),
                        row: reference,
                    });
                    continue;
                }
                // NOTE: `mapping_guidelines.md` §Message Spreadsheet: a sub-path such as
                // `Observation[2].note` repeats what the segment map's resource-rooted rows
                // already write, so the segment map runs at the resource root.
                None => base,
            };
            ran = true;
            self.segment(&scope, segment_map, &base, &mut named)?;
        }
        if ran {
            self.unmapped_fields(&scope, &named);
        }
        Ok(())
    }

    /// Evaluates a row's condition, recording why a row does not apply.
    ///
    /// # Errors
    ///
    /// Returns [`MapError::Stopped`] when a `NOT VALUED ERROR` check holds.
    fn gate(&mut self, scope: &Scope<'_>, row: &Row, reference: &RowRef) -> Result<bool, MapError> {
        match &row.condition {
            Condition::Always => Ok(true),
            Condition::Narrative => {
                self.outcomes.push(Outcome::NarrativeCondition {
                    at: scope.at.clone(),
                    row: reference.clone(),
                });
                Ok(false)
            }
            Condition::Unsupported { text, .. } => {
                self.outcomes.push(Outcome::UnsupportedCondition {
                    at: scope.at.clone(),
                    row: reference.clone(),
                    text: text.clone(),
                });
                Ok(false)
            }
            Condition::Computable(expr) => {
                match condition::evaluate(expr, &mut |operand| scope.probe(operand)) {
                    Ok(holds) => Ok(holds),
                    Err(Unevaluable::Operand(operand)) => {
                        self.outcomes.push(Outcome::UnevaluableCondition {
                            at: scope.at.clone(),
                            row: reference.clone(),
                            operand,
                        });
                        Ok(false)
                    }
                    Err(Unevaluable::Stop(operand)) => Err(MapError::Stopped {
                        at: Box::new(scope.at.clone()),
                        row: Box::new(reference.clone()),
                        operand,
                    }),
                }
            }
        }
    }

    /// Runs a segment map over the scope's segment into `base`.
    fn segment(
        &mut self,
        scope: &Scope<'a>,
        map: &'a Map,
        base: &Base,
        named: &mut BTreeSet<usize>,
    ) -> Result<(), MapError> {
        let Some(segment) = scope.segment() else {
            return Ok(());
        };
        for row in &map.rows {
            let reference = row_ref(map, row);
            let (id, path) = split_source(&row.source, '-');
            if id != segment.id() {
                self.outcomes.push(Outcome::UnsupportedShape {
                    at: scope.at.clone(),
                    row: reference,
                });
                continue;
            }
            let Some((&position, components)) = path.split_first() else {
                if self.gate(scope, row, &reference)? {
                    self.apply(scope, row, &reference, Datum::Segment, 0, base)?;
                }
                continue;
            };
            named.insert(position);
            let Some(field) = segment.field(position).filter(|field| field.is_valued()) else {
                continue;
            };
            // NOTE: no specification governs this: our own design; a computable condition
            // is evaluated per repetition, reading the repetition mapped, and any other
            // gate is met once per row, so a narrative row counts once.
            let computable = matches!(row.condition, Condition::Computable(_));
            let mut once = scope.clone();
            once.at = scope.at.clone().with_field(position);
            if !computable && !self.gate(&once, row, &reference)? {
                continue;
            }
            let source_type = row
                .source_type
                .clone()
                .or_else(|| definition_type(segment.id(), position));
            let typed = Row {
                source_type: resolve_varies(source_type, segment),
                ..row.clone()
            };
            for (repetition, value) in field.repetitions().iter().enumerate() {
                let datum = Datum::Repetition(value).at(components);
                if !datum.valued() {
                    continue;
                }
                let mut inner = scope.clone();
                inner.at = Location {
                    repetition: Some(repetition.saturating_add(1)),
                    ..once.at.clone()
                };
                inner.repetition = Some((position, repetition));
                if computable && !self.gate(&inner, row, &reference)? {
                    continue;
                }
                self.apply(&inner, &typed, &reference, datum, repetition, base)?;
            }
        }
        Ok(())
    }

    /// Counts the valued fields of the scope's segment no applied row names.
    fn unmapped_fields(&mut self, scope: &Scope<'a>, named: &BTreeSet<usize>) {
        let Some(segment) = scope.segment() else {
            return;
        };
        for (offset, field) in segment.fields().iter().enumerate() {
            let position = offset.saturating_add(1);
            let header = segment.id() == "MSH" && position <= 2;
            if field.is_valued() && !header && !named.contains(&position) {
                self.outcomes.push(Outcome::UnmappedField {
                    at: scope.at.clone().with_field(position),
                });
            }
        }
    }

    /// Runs a data type map over `datum` into `base`.
    fn datatype(
        &mut self,
        scope: &Scope<'a>,
        map: &'a Map,
        datatype: &str,
        datum: Datum<'a>,
        base: &Base,
    ) -> Result<(), MapError> {
        let mut inner = scope.clone();
        inner.datatype = Some((String::from(datatype), datum));
        inner.repetition = None;
        let mut named = BTreeSet::new();
        for row in &map.rows {
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
                named.insert(*first);
            }
            let part = datum.at(&path);
            if !part.valued() {
                continue;
            }
            if !self.gate(&inner, row, &reference)? {
                continue;
            }
            self.apply(&inner, row, &reference, part, 0, base)?;
        }
        for component in datum.valued_parts() {
            if !named.contains(&component) {
                self.outcomes.push(Outcome::UnmappedComponent {
                    at: scope.at.clone(),
                    map: map.id.clone(),
                    component,
                });
            }
        }
        Ok(())
    }

    /// Applies one row whose condition holds to a valued `datum`.
    fn apply(
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
                let target = steps.last().and_then(|step| step.reference.clone());
                let inner = target
                    .as_ref()
                    .map(|target| target.path.clone())
                    .unwrap_or_default();
                if inner.is_empty() && row.assignment.is_none() && row.mapped_via.is_none() {
                    // NOTE: no specification governs this: our own design; a resource no data
                    // type map can fill is never created, so no empty resource enters the Bundle.
                    let source_type = row.source_type.clone().unwrap_or_default();
                    let resource = target.map(|target| target.resource).unwrap_or_default();
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

    /// Writes the value of `datum` at `leaf`: an assignment, a translation,
    /// a data type map or a converted primitive.
    fn value(
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
        match &row.assignment {
            Some(Assignment::Literal(text)) => {
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
                return Ok(());
            }
            Some(Assignment::Unsupported(text)) => {
                self.outcomes.push(Outcome::UnsupportedAssignment {
                    at: at.clone(),
                    row: reference.clone(),
                    text: text.clone(),
                });
                return Ok(());
            }
            None => {}
        }
        if let Some(mapped_via) = &row.mapped_via {
            self.table(leaf, &resolved, mapped_via, datum, at, reference);
            return Ok(());
        }
        let source_type = row.source_type.clone().unwrap_or_default();
        let target_type = match resolved.location() {
            fhirconnect::tree::element::Location::Complex(schema) => String::from(schema.name),
            _ => String::from(
                resolved
                    .type_code()
                    .or_else(|| alternative(&resolved))
                    .unwrap_or_default(),
            ),
        };
        let complex = matches!(
            resolved.location(),
            fhirconnect::tree::element::Location::Complex(_)
        );
        let temporal = TEMPORAL_SOURCES.contains(&source_type.as_str())
            && TEMPORAL_TARGETS.contains(&target_type.as_str());
        if !temporal && !source_type.is_empty() {
            match self.corpus.find("datatype", &source_type, &target_type) {
                Ok(map) => return self.datatype(scope, map, &source_type, datum, leaf),
                Err(_) if complex => {
                    self.outcomes.push(Outcome::NoDatatypeMap {
                        at: at.clone(),
                        row: reference.clone(),
                        source_type,
                        target_type,
                    });
                    return Ok(());
                }
                Err(_) => {}
            }
        }
        if complex {
            self.outcomes.push(Outcome::NoDatatypeMap {
                at: at.clone(),
                row: reference.clone(),
                source_type,
                target_type,
            });
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

    /// Records one write against the leaf's resource.
    fn record(&mut self, leaf: &Base, value: Pending, at: &Location, reference: &RowRef) {
        if let Some(resource) = self.resources.get_mut(leaf.resource) {
            resource.writes.push(Write {
                slots: leaf.slots.clone(),
                value,
                at: at.clone(),
                row: reference.clone(),
            });
        }
    }

    /// Resolves a leaf's path against its resource type.
    fn resolve(
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

    /// Extends `base` by `steps`, giving each step its instance key: its label,
    /// and on the first repeating step the source repetition.
    fn slots(
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
    /// the base's last repeating element for any `k` but 1.
    fn instance(
        &mut self,
        base: &Base,
        instance: Option<&Label>,
        at: &Location,
        reference: &RowRef,
    ) -> Option<Base> {
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

    /// Creates, or finds, the resource a `(Type)` reference at the end of
    /// `steps` names, and records the reference to it.
    fn reference(
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
    fn message_resource(
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
    fn create(&mut self, resource_type: &str, identity: Identity) -> usize {
        let index = self.resources.len();
        let control = self.parsed.message().control_id().unwrap_or_default();
        self.resources.push(Resource {
            type_name: String::from(resource_type),
            identity,
            full_url: full_url(control, index, resource_type),
            writes: Vec::new(),
        });
        index
    }

    /// Asks the terminology server for every recorded translation.
    ///
    /// # Errors
    ///
    /// Returns [`MapError::Terminology`] when the server refuses a
    /// translation or cannot be reached.
    pub(super) async fn translate(
        &mut self,
        terminology: Option<&ferrobridge_term::client::Client>,
    ) -> Result<usize, MapError> {
        let mut calls = 0usize;
        let mut outcomes = Vec::new();
        for request in &mut self.requests {
            let Some(client) = terminology else {
                outcomes.push(Outcome::NoTerminology {
                    at: request.at.clone(),
                    row: request.row.clone(),
                });
                continue;
            };
            for group in &request.groups {
                calls = calls.saturating_add(1);
                let answer = client
                    .translate(
                        &group.source,
                        &request.code,
                        &group.target,
                        Some(&request.url),
                    )
                    .await
                    .map_err(|source| MapError::Terminology {
                        concept_map: request.url.clone(),
                        source: Box::new(source),
                    })?;
                if let Some(found) = answer.accepted().find_map(|found| found.concept.clone()) {
                    request.answer = Some(found);
                    break;
                }
            }
            if request.answer.is_none() {
                outcomes.push(Outcome::Untranslated {
                    at: request.at.clone(),
                    row: request.row.clone(),
                    concept_map: request.url.clone(),
                });
            }
        }
        self.outcomes.extend(outcomes);
        Ok(calls)
    }

    /// Applies every write and assembles the Bundle.
    pub(super) fn finish(mut self) -> (Value, Vec<Outcome>) {
        let mut documents = Vec::new();
        let resources = core::mem::take(&mut self.resources);
        for resource in &resources {
            let document = self.build(resource);
            documents.push((resource, document));
        }
        let mut entries: Vec<(&Resource, Value)> = Vec::new();
        let mut envelope = None;
        for (resource, document) in documents {
            if resource.identity == Identity::Envelope {
                envelope = Some(document);
                continue;
            }
            if let Value::Object(object) = &document {
                let decoded = fhir_types::r4::resource::Resource::from_json(
                    object,
                    &mut fhir_types::codec::Path::root(&resource.type_name),
                );
                if let Err(error) = decoded {
                    self.outcomes.push(Outcome::Undecodable {
                        resource: resource.type_name.clone(),
                        full_url: resource.full_url.clone(),
                        error,
                    });
                }
            }
            entries.push((resource, document));
        }
        entries.sort_by_key(|(resource, _)| resource.type_name != "MessageHeader");
        let mut bundle = envelope.unwrap_or_else(|| document_of("Bundle"));
        for (index, (resource, document)) in entries.into_iter().enumerate() {
            let occurrence = Occurrence::new([index]);
            for (path, value) in [
                (
                    "$resource.entry.fullUrl",
                    Value::String(resource.full_url.clone()),
                ),
                ("$resource.entry.resource", document),
            ] {
                let written = path
                    .parse::<FhirPath>()
                    .ok()
                    .map(|path| write(&SCHEMAS, &mut bundle, &path, &occurrence, value));
                if let Some(Err(error)) = written {
                    tracing::warn!(error = %error, "a Bundle entry could not be written");
                }
            }
        }
        (bundle, self.outcomes)
    }

    /// Applies one resource's writes in order.
    fn build(&mut self, resource: &Resource) -> Value {
        let mut document = document_of(&resource.type_name);
        let mut indices: BTreeMap<(Vec<usize>, String, String), usize> = BTreeMap::new();
        let mut lengths: BTreeMap<(Vec<usize>, String), usize> = BTreeMap::new();
        let mut written: BTreeSet<(String, Vec<usize>)> = BTreeSet::new();
        for pending in &resource.writes {
            let Some(value) = self.settle(&pending.value) else {
                continue;
            };
            let names: Vec<&str> = pending
                .slots
                .iter()
                .map(|slot| slot.name.as_str())
                .collect();
            let flags = match repeating(&resource.type_name, &names) {
                Ok(flags) => flags,
                Err(error) => {
                    self.outcomes.push(Outcome::UnknownElement {
                        at: pending.at.clone(),
                        row: pending.row.clone(),
                        error,
                    });
                    continue;
                }
            };
            let mut occurrence = Vec::new();
            let mut fresh = Vec::new();
            let mut prefix = String::new();
            for (slot, repeats) in pending.slots.iter().zip(flags) {
                prefix.push('.');
                prefix.push_str(&slot.name);
                if !repeats {
                    continue;
                }
                let key = (occurrence.clone(), prefix.clone(), slot.key.clone());
                let index = if let Some(index) = indices.get(&key) {
                    *index
                } else {
                    let length = (occurrence.clone(), prefix.clone());
                    let next = lengths.get(&length).copied().unwrap_or_default();
                    fresh.push((key, length, next));
                    next
                };
                occurrence.push(index);
            }
            let leaf = (prefix.clone(), occurrence.clone());
            if written.contains(&leaf) {
                self.outcomes.push(Outcome::Superseded {
                    at: pending.at.clone(),
                    row: pending.row.clone(),
                });
                continue;
            }
            let path = format!("$resource{prefix}");
            let Ok(path) = path.parse::<FhirPath>() else {
                continue;
            };
            match write(
                &SCHEMAS,
                &mut document,
                &path,
                &Occurrence::new(occurrence),
                value,
            ) {
                Ok(()) => {
                    for (key, length, next) in fresh {
                        indices.insert(key, next);
                        lengths.insert(length, next.saturating_add(1));
                    }
                    written.insert(leaf);
                }
                Err(error) => self.outcomes.push(Outcome::Unwritable {
                    at: pending.at.clone(),
                    row: pending.row.clone(),
                    error,
                }),
            }
        }
        document
    }

    /// The value a pending write carries once the translations are in.
    fn settle(&self, pending: &Pending) -> Option<Value> {
        match pending {
            Pending::Ready(value) => Some(value.clone()),
            Pending::Translation { request, part } => {
                let concept = self.requests.get(*request)?.answer.as_ref()?;
                let text = match part {
                    Part::Code => concept.code.clone(),
                    Part::System => concept.system.clone(),
                    Part::Display => concept.display.clone(),
                }?;
                Some(match text.as_str() {
                    "true" if *part == Part::Code => Value::Bool(true),
                    "false" if *part == Part::Code => Value::Bool(false),
                    _ => Value::String(text),
                })
            }
        }
    }
}

impl Base {
    /// This base extended by one step.
    fn child(&self, name: &str, key: &str) -> Self {
        let mut slots = self.slots.clone();
        slots.push(Slot {
            name: String::from(name),
            key: String::from(key),
        });
        Self {
            resource: self.resource,
            slots,
        }
    }
}

/// An empty document of `resource_type`.
fn document_of(resource_type: &str) -> Value {
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
fn full_url(control: &str, ordinal: usize, resource_type: &str) -> String {
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

/// Resolves `$resource.<names>` against `resource_type`.
fn resolve_path(
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

/// The type of the choice alternative a path ends on, read from the element
/// table's entry: the listed type whose name the JSON key's suffix spells
/// (<https://hl7.org/fhir/R4/json.html>, choice elements).
///
/// `Resolved::type_code` answers `None` for a resolved choice alternative,
/// since the table entry lists every alternative.
fn alternative(resolved: &fhirconnect::tree::element::Resolved) -> Option<&'static str> {
    // TODO(#290): read the alternative from fhirconnect once `Resolved` names it.
    let Some(Move::Member(field)) = resolved.moves().last() else {
        return None;
    };
    let stem = field.path().rsplit('.').next()?.strip_suffix("[x]")?;
    let suffix = field.key().strip_prefix(stem)?;
    field.types().iter().copied().find(|code| {
        let mut characters = code.chars();
        characters.next().is_some_and(|first| {
            suffix.starts_with(first.to_ascii_uppercase())
                && suffix.get(1..) == Some(characters.as_str())
        })
    })
}

/// Whether each step of `names` enters a repeating element.
fn repeating(resource_type: &str, names: &[&str]) -> Result<Vec<bool>, ElementError> {
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

/// Splits a source code into its id and positions: `PID-11.9` into `PID` and
/// `[11, 9]`, `CX.1` into `CX` and `[1]`, `MSG` into `MSG` and `[]`.
fn split_source(source: &str, separator: char) -> (&str, Vec<usize>) {
    match source.split_once(separator) {
        Some((id, rest)) => (
            id,
            rest.split('.')
                .map_while(|part| part.parse().ok())
                .collect(),
        ),
        None => (source, Vec::new()),
    }
}

/// The data type the definitions give a field, when its `TypeInfo` names
/// none.
fn definition_type(segment: &str, position: usize) -> Option<String> {
    hl7v2_types::segment::find(segment)?
        .fields
        .iter()
        .find(|field| usize::from(field.position) == position)?
        .data_type
        .map(|data_type| String::from(data_type.code()))
}

/// The type a `varies` field holds: OBX-5 is of the type OBX-2 names.
///
/// NOTE: HL7 v2.5.1 chapter 7 §7.4.2.2: OBX-2 "contains the format of the
/// observation value in OBX", the one `varies` field the mapped segments carry.
fn resolve_varies(source_type: Option<String>, segment: &Segment) -> Option<String> {
    match source_type.as_deref() {
        Some("varies" | "Varies") if segment.id() == "OBX" => segment.text(2).map(String::from),
        _ => source_type,
    }
}

/// The reference of a row, for an outcome.
fn row_ref(map: &Map, row: &Row) -> RowRef {
    RowRef {
        map: map.id.clone(),
        source: row.source.clone(),
        target: row.target_code.clone(),
    }
}

/// One placed segment with what the message map matches it by.
#[derive(Debug, Clone)]
struct Visit<'p> {
    index: usize,
    code: String,
    top_follow: Vec<&'static str>,
    key: Vec<usize>,
    chain: Vec<&'p [Item]>,
}

impl Visit<'_> {
    /// Whether a message map row's source code names this segment.
    ///
    /// `STRUCT.GROUP.SEG` names the segment at that group path. The corpus
    /// also writes `STRUCT:follow:PREV.SEG` for a top-level segment that
    /// follows `PREV`; no specification defines that form, so our own design
    /// reads it as the top-level node with a `PREV` segment node among the
    /// siblings before it, back to the previous node of its own segment id.
    fn matches(&self, source: &str) -> bool {
        if source == self.code {
            return true;
        }
        let Some((structure, rest)) = source.split_once(":follow:") else {
            return false;
        };
        let Some((previous, segment)) = rest.split_once('.') else {
            return false;
        };
        self.top_follow.contains(&previous) && self.code == format!("{structure}.{segment}")
    }
}

/// Collects every placed segment in message order.
fn collect<'p>(
    items: &'p [Item],
    structure: &str,
    path: &mut Vec<&'static str>,
    key: &mut Vec<usize>,
    chain: &mut Vec<&'p [Item]>,
    top: &'static [hl7v2_types::model::Node],
    visits: &mut Vec<Visit<'p>>,
) {
    chain.insert(0, items);
    for item in items {
        match item {
            Item::Segment(placed) => {
                let mut segment_key = key.clone();
                if repeats(placed.node.cardinality.max) {
                    segment_key.push(placed.occurrence);
                }
                let mut code = String::from(structure);
                for name in path.iter() {
                    code.push('.');
                    code.push_str(name);
                }
                code.push('.');
                code.push_str(placed.node.segment.id);
                let top_follow = if path.is_empty() {
                    preceding(top, placed.node.id, placed.node.segment.id)
                } else {
                    Vec::new()
                };
                visits.push(Visit {
                    index: placed.index,
                    code,
                    top_follow,
                    key: segment_key,
                    chain: chain.clone(),
                });
            }
            Item::Group(instance) => {
                path.push(instance.group.name);
                let pushed = repeats(instance.group.cardinality.max);
                if pushed {
                    key.push(instance.occurrence);
                }
                collect(&instance.items, structure, path, key, chain, top, visits);
                if pushed {
                    key.pop();
                }
                path.pop();
            }
        }
    }
    chain.remove(0);
}

/// The ids of the top-level segments before the node `id`, back to the
/// previous node of the same segment id.
fn preceding(
    nodes: &'static [hl7v2_types::model::Node],
    id: &str,
    segment: &str,
) -> Vec<&'static str> {
    let Some(position) = nodes.iter().position(
        |node| matches!(node, hl7v2_types::model::Node::Segment(reference) if reference.id == id),
    ) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for node in nodes.get(..position).unwrap_or_default().iter().rev() {
        if let hl7v2_types::model::Node::Segment(reference) = node {
            if reference.segment.id == segment {
                break;
            }
            ids.push(reference.segment.id);
        }
    }
    ids
}

/// Whether a node repeats.
const fn repeats(max: Max) -> bool {
    match max {
        Max::Unbounded => true,
        Max::Bounded(bound) => bound > 1,
    }
}
