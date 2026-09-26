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
use crate::map::constraint;
use crate::map::convert;
use crate::map::corpus::{Assignment, Condition, Corpus, Kind, Map, MappedVia, Row, TableGroup};
use crate::map::notation::{self, Label, Step, Target};
use crate::map::{ElementError, MapError, Outcome, RowRef};
use crate::parse::{Component, Field, Item, Location, Parsed, Repetition, Segment};

/// The v2 date and time types the mapping guidelines convert to FHIR date and
/// time types (`mapping_guidelines.md` §Data Type Spreadsheet).
const TEMPORAL_SOURCES: [&str; 4] = ["DTM", "DT", "TS", "TM"];

/// The FHIR types those conversions reach.
const TEMPORAL_TARGETS: [&str; 4] = ["date", "dateTime", "instant", "time"];

/// The `MessageHeader` endpoints an MSH facility field fills when the fields
/// the guide's MSH map writes them from are empty: the target, those fields,
/// and the facility field.
///
/// No specification governs this: our own design. The guide's MSH-3 and
/// MSH-24 rows leave a message valuing neither to the implementer, and FHIR R4
/// requires `MessageHeader.source` and `destination.endpoint`
/// (<https://hl7.org/fhir/R4/messageheader.html>).
const FACILITY_ENDPOINTS: [(&str, [usize; 2], usize); 2] = [
    ("source[1].endpoint", [3, 24], 4),
    ("destination[1].endpoint", [5, 25], 6),
];

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
    definition: &'static hl7v2_types::model::Segment,
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

/// The text of a concatenation read in `scope`: each literal as written and
/// each operand's first value, joined.
///
/// No specification governs an operand with no value: our own design joins
/// the empty text for it, since the row's own source is valued when a row
/// applies and the guide's joins name the other components beside it.
///
/// # Errors
///
/// Returns the operand the scope cannot read.
fn concatenate(scope: &Scope<'_>, parts: &[condition::Part]) -> Result<String, String> {
    let mut text = String::new();
    for part in parts {
        match part {
            condition::Part::Literal(literal) => text.push_str(literal),
            condition::Part::Operand(operand) => {
                let read = scope.probe(operand).ok_or_else(|| operand.to_string())?;
                text.push_str(read.text.as_deref().unwrap_or_default());
            }
        }
    }
    Ok(text)
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

/// What one data type map wrote under one child of a complex target: the
/// indices of its writes, and each write's steps below the target with its
/// value, `None` for a translation still pending.
#[derive(Debug)]
struct Written {
    map: String,
    indices: Vec<usize>,
    values: Vec<(Vec<Slot>, Option<Value>)>,
}

/// One recorded write.
#[derive(Debug, Clone)]
struct Write {
    slots: Vec<Slot>,
    value: Pending,
    at: Location,
    row: RowRef,
    /// Whether the bridge derived the value where no row of the guide writes
    /// one ([`Run::endpoint`]), so a row of the guide at the same element
    /// replaces it.
    fallback: bool,
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

/// The lengths of a run's records at one point: the writes per resource,
/// whose count is the resource count, the requests and the outcomes.
#[derive(Debug)]
struct Mark {
    writes: Vec<usize>,
    requests: usize,
    outcomes: usize,
}

/// The components a data type map's rows name, and those whose rows wrote.
#[derive(Debug, Default)]
struct Covered {
    named: BTreeSet<usize>,
    wrote: BTreeSet<usize>,
}

impl Covered {
    /// Counts `started`'s component as written when the run's write count
    /// grew past the count its row started at.
    fn settle(&mut self, started: Option<(usize, usize)>, pushed: usize) {
        if let Some((component, before)) = started
            && pushed > before
        {
            self.wrote.insert(component);
        }
    }
}

/// One write recorded since a [`Mark`], as far as two maps' writes are
/// compared: the resource, the steps and the value or the translation.
#[derive(Debug, PartialEq)]
struct Recorded {
    resource: usize,
    type_name: String,
    slots: Vec<Slot>,
    value: Option<Value>,
    translation: Option<(String, String, Part)>,
}

/// A run in progress.
#[derive(Debug)]
pub(super) struct Run<'a> {
    corpus: &'a Corpus,
    parsed: &'a Parsed,
    resources: Vec<Resource>,
    requests: Vec<Request>,
    outcomes: Vec<Outcome>,
    /// The count of writes recorded over the run, which a rollback leaves as
    /// it is, so its growth shows that a row wrote.
    pushed: usize,
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
            pushed: 0,
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

    /// The message map rows naming a placed segment: the rows whose source is
    /// its path, else the rows of the one source [`regrouped`] reaches it
    /// by, counted as `group-path` when a group is added or omitted and as
    /// `group-renamed` per group paired under another name; `None` when no
    /// row names it.
    fn rows(&mut self, map: &'a Map, visit: &Visit<'a>, at: &Location) -> Option<Vec<&'a Row>> {
        let rows: Vec<&Row> = map
            .rows
            .iter()
            .filter(|row| visit.matches(&row.source))
            .collect();
        if !rows.is_empty() {
            return Some(rows);
        }
        let structure = self.parsed.structure_name();
        let nodes = self.parsed.structure().nodes;
        let tree = tree_paths(structure, nodes);
        let contents = Contents {
            rows: row_contents(map),
            tree: tree_contents(structure, nodes),
        };
        let found = regrouped(map, &visit.code, &tree, &contents)?;
        if found.shifted {
            self.outcomes.push(Outcome::GroupPath {
                at: at.clone(),
                row_path: String::from(found.source),
                tree_path: visit.code.clone(),
            });
        }
        for (row_group, tree_group) in found.renamed {
            self.outcomes.push(Outcome::GroupRenamed {
                at: at.clone(),
                row_path: String::from(found.source),
                tree_path: visit.code.clone(),
                row_group,
                tree_group,
            });
        }
        Some(
            map.rows
                .iter()
                .filter(|row| row.source == found.source)
                .collect(),
        )
    }

    /// Maps one placed segment through the message map rows naming it.
    fn visit(&mut self, map: &'a Map, visit: &Visit<'a>) -> Result<(), MapError> {
        let at = self.parsed.location(visit.index);
        let Some(rows) = self.rows(map, visit, &at) else {
            self.outcomes.push(Outcome::UnmappedSegment { at });
            return Ok(());
        };
        let scope = Scope {
            parsed: self.parsed,
            index: visit.index,
            definition: visit.definition,
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
            self.facility_endpoints(&scope, segment_map, &base);
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
            Condition::Defective { text } => {
                self.outcomes.push(Outcome::DefectiveCondition {
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
            let own = Operand::Field {
                segment: String::from(id),
                path: path.clone(),
            };
            let Some(field) = segment.field(position).filter(|field| field.is_valued()) else {
                let mut empty = scope.clone();
                empty.at = scope.at.clone().with_field(position);
                self.absence(&empty, row, &reference, &own, base)?;
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
            let legacy = self.parsed.structure().withdrawn_as_of.is_some() && components.is_empty();
            if (legacy || row.source_type.is_none())
                && let Some((code, base)) =
                    version_specific(self.corpus, scope.definition, position)
            {
                self.outcomes.push(Outcome::BaseTyped {
                    at: once.at.clone(),
                    row: reference.clone(),
                    field: format!("{id}-{position}"),
                    code: code.code,
                    base: base.code,
                    version: code.version,
                });
            }
            let (source_type, replaced) = field_type(
                row.source_type.clone(),
                definition_type(self.corpus, scope.definition, position),
                legacy,
            );
            if let (Some(row_type), Some(version_type)) = (replaced, source_type.clone()) {
                self.outcomes.push(Outcome::VersionTyped {
                    at: once.at.clone(),
                    row: reference.clone(),
                    field: format!("{id}-{position}"),
                    row_type,
                    version_type,
                    version: self.parsed.structure().version,
                });
            }
            let typed = Row {
                source_type: resolve_varies(source_type, segment),
                ..row.clone()
            };
            for (repetition, value) in field.repetitions().iter().enumerate() {
                let datum = Datum::Repetition(value).at(components);
                let mut inner = scope.clone();
                inner.at = Location {
                    repetition: Some(repetition.saturating_add(1)),
                    ..once.at.clone()
                };
                inner.repetition = Some((position, repetition));
                if !datum.valued() {
                    self.absence(&inner, row, &reference, &own, base)?;
                    continue;
                }
                if computable && !self.gate(&inner, row, &reference)? {
                    continue;
                }
                self.apply(&inner, &typed, &reference, datum, repetition, base)?;
            }
        }
        Ok(())
    }

    /// Applies a row to its empty source when the row maps that absence: it
    /// assigns a value, and its condition requires `own`, its source, not
    /// valued ([`condition::requires_absent`]) and holds.
    ///
    /// Any other row on an empty source maps nothing and is skipped
    /// (`mapping_guidelines.md` §General Format/Approach: a condition decides
    /// whether the v2 element is mapped).
    ///
    /// # Errors
    ///
    /// Returns [`MapError::Stopped`] when a `NOT VALUED ERROR` check holds.
    fn absence(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        own: &Operand,
        base: &Base,
    ) -> Result<(), MapError> {
        let maps_absence = row.assignment.is_some()
            && matches!(&row.condition, Condition::Computable(expr)
                if condition::requires_absent(expr, own));
        if maps_absence && self.gate(scope, row, reference)? {
            self.apply(scope, row, reference, Datum::Absent, 0, base)?;
        }
        Ok(())
    }

    /// Writes each `MessageHeader` endpoint of [`FACILITY_ENDPOINTS`] whose
    /// fields are empty from the first valued repetition of its facility
    /// field, by the rule of [`Run::endpoint`], and counts it.
    ///
    /// The guide's own rows for those empty fields write a data-absent-reason
    /// extension on the endpoint ([`Run::absence`]); the facility's value
    /// replaces those writes, so an endpoint never carries both.
    fn facility_endpoints(&mut self, scope: &Scope<'a>, map: &'a Map, base: &Base) {
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

    /// Runs a data type map over `datum` into `base`, counting each valued
    /// component neither its rows nor `also` name.
    fn datatype(
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
    fn datatypes(
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
                        Pending::Translation { .. } => None,
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
    fn mark(&self) -> Mark {
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
    fn rollback(&mut self, mark: &Mark) {
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
    fn datatype_rows(
        &mut self,
        scope: &Scope<'a>,
        map: &'a Map,
        datatype: &str,
        datum: Datum<'a>,
        base: &Base,
        claimed: &BTreeSet<String>,
    ) -> Result<Covered, MapError> {
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

    /// Narrows each set of halves of one value among `maps`
    /// ([`Map::half_of`]) to the one the row's place names, and returns the
    /// maps with the components the halves left out read.
    ///
    /// The guide maps an `EIP` by two rows from one source into one element
    /// with two labels (`SPM-2` into `identifier[1]` and `identifier[2]`), so
    /// the rows in map order take the halves in component order, each
    /// counted as [`Outcome::DatatypeHalf`]. When the row has no such
    /// siblings, or the value holds a component no half reads (an `EI`
    /// written where the row names an `EIP`), every half is kept and
    /// [`Run::datatypes`] refuses the row. No specification governs this:
    /// our own design.
    fn halve(
        &mut self,
        scope: &Scope<'a>,
        row: &Row,
        reference: &RowRef,
        maps: Vec<&'a Map>,
        datum: Datum<'a>,
    ) -> (Vec<&'a Map>, BTreeSet<usize>) {
        let place = self.place(row, reference);
        let valued = datum.valued_parts();
        let mut kept = Vec::new();
        let mut also = BTreeSet::new();
        for mut set in group(maps, Map::half_of) {
            if set.len() < 2 {
                kept.extend(set);
                continue;
            }
            set.sort_by_key(|map| (map.components().first().copied(), map.id.clone()));
            let read: BTreeSet<usize> = set.iter().flat_map(|map| map.components()).collect();
            let chosen = place
                .filter(|(_, count)| *count == set.len())
                .filter(|_| valued.iter().all(|component| read.contains(component)))
                .and_then(|(index, _)| set.get(index).copied());
            let Some(chosen) = chosen else {
                kept.extend(set);
                continue;
            };
            for other in set.iter().filter(|other| other.id != chosen.id) {
                also.extend(other.components());
            }
            self.outcomes.push(Outcome::DatatypeHalf {
                at: scope.at.clone(),
                row: reference.clone(),
                map: chosen.id.clone(),
                halves: set.iter().map(|map| map.id.clone()).collect(),
            });
            kept.push(chosen);
        }
        kept.sort_by(|a, b| a.id.cmp(&b.id));
        (kept, also)
    }

    /// The place of `row` among the rows of its map from the same source
    /// whose targets differ from its own only in their labels, with their
    /// count.
    fn place(&self, row: &Row, reference: &RowRef) -> Option<(usize, usize)> {
        let map = self.corpus.get(&reference.map)?;
        let shape = unlabelled(&row.target_code);
        let siblings: Vec<&Row> = map
            .rows
            .iter()
            .filter(|other| {
                other.source == row.source
                    && other.assignment.is_none()
                    && other.mapped_via.is_none()
                    && unlabelled(&other.target_code) == shape
            })
            .collect();
        let index = siblings
            .iter()
            .position(|other| other.target_code == row.target_code)?;
        Some((index, siblings.len()))
    }

    /// Chooses one map from each set of alternatives among `maps`
    /// ([`Map::alternative_to`]) and keeps every other map, by id.
    ///
    /// The map of a set whose rows write from the most components of `datum`
    /// is chosen; among equally specific maps whose writes agree, the one
    /// with the fewest rows, the plain map. Equally specific maps whose
    /// writes differ are counted as [`Outcome::DatatypeAmbiguous`], and none
    /// of them runs. No specification governs this: our own design, since
    /// no row of the guide names the variant it runs.
    fn select(
        &mut self,
        scope: &Scope<'a>,
        reference: &RowRef,
        maps: Vec<&'a Map>,
        datatype: &str,
        datum: Datum<'a>,
        leaf: &Base,
    ) -> Result<Vec<&'a Map>, MapError> {
        let mut chosen = Vec::new();
        for set in group(maps, Map::alternative_to) {
            match set.as_slice() {
                [one] => chosen.push(*one),
                several => {
                    if let Some(map) =
                        self.choose(scope, reference, several, datatype, datum, leaf)?
                    {
                        chosen.push(map);
                    }
                }
            }
        }
        chosen.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(chosen)
    }

    /// Chooses one of the alternative maps `set` for `datum`, as
    /// [`Run::select`] describes, running each on trial and rolling it back.
    fn choose(
        &mut self,
        scope: &Scope<'a>,
        reference: &RowRef,
        set: &[&'a Map],
        datatype: &str,
        datum: Datum<'a>,
        leaf: &Base,
    ) -> Result<Option<&'a Map>, MapError> {
        let mut trials = Vec::new();
        for map in set {
            let mark = self.mark();
            let covered =
                self.datatype_rows(scope, map, datatype, datum, leaf, &BTreeSet::new())?;
            let recorded = self.recorded_since(&mark);
            self.rollback(&mark);
            trials.push((*map, covered.wrote.len(), recorded));
        }
        let most = trials.iter().map(|(_, count, _)| *count).max().unwrap_or(0);
        trials.retain(|(_, count, _)| *count == most);
        let agree = trials
            .split_first()
            .is_some_and(|((_, _, first), rest)| rest.iter().all(|(_, _, other)| other == first));
        if agree {
            return Ok(trials.iter().map(|(map, _, _)| *map).min_by(|a, b| {
                a.rows
                    .len()
                    .cmp(&b.rows.len())
                    .then_with(|| a.id.cmp(&b.id))
            }));
        }
        self.outcomes.push(Outcome::DatatypeAmbiguous {
            at: scope.at.clone(),
            row: reference.clone(),
            candidates: trials.iter().map(|(map, _, _)| map.id.clone()).collect(),
        });
        Ok(None)
    }

    /// The writes recorded since `mark`, in every resource, for comparison.
    fn recorded_since(&self, mark: &Mark) -> Vec<Recorded> {
        let mut recorded = Vec::new();
        for (index, resource) in self.resources.iter().enumerate() {
            let start = mark.writes.get(index).copied().unwrap_or(0);
            for write in resource.writes.get(start..).unwrap_or_default() {
                let (value, translation) = match &write.value {
                    Pending::Ready(value) => (Some(value.clone()), None),
                    Pending::Translation { request, part } => (
                        None,
                        self.requests
                            .get(*request)
                            .map(|asked| (asked.url.clone(), asked.code.clone(), *part)),
                    ),
                };
                recorded.push(Recorded {
                    resource: index,
                    type_name: resource.type_name.clone(),
                    slots: write.slots.clone(),
                    value,
                    translation,
                });
            }
        }
        recorded
    }

    /// The data type maps from `source_type` into the complex element `leaf`:
    /// those named for its type, else those named for its element path.
    ///
    /// The guide's map titles name a target by type or by element path
    /// (`mapping_guidelines.md`, the `CQ[ServiceRequest.duration]` title
    /// form), and a backbone element such as `MessageHeader.source` has no
    /// type name, so its maps carry the path, as
    /// `datatype-hd-endpoint-to-messageheader-source`.
    fn complex_maps(&self, source_type: &str, target_type: &str, leaf: &Base) -> Vec<&'a Map> {
        let corpus = self.corpus;
        let named = corpus.qualifying("datatype", source_type, target_type);
        if !named.is_empty() {
            return named;
        }
        let Some(resource) = self.resources.get(leaf.resource) else {
            return Vec::new();
        };
        let mut path = resource.type_name.clone();
        for slot in &leaf.slots {
            path.push('-');
            path.push_str(&slot.name);
        }
        corpus.qualifying("datatype", source_type, &path)
    }

    /// Writes a v2 `HD` into a FHIR `url` element, with its namespace ID in
    /// the `name` element beside it when there is one.
    ///
    /// The url is the derived form of [`convert::endpoint`], written as a
    /// fallback ([`Run::write`]): where a row of the guide writes the
    /// endpoint, as `datatype-hd-endpoint-to-messageheader-source` does for a
    /// typed universal ID, that row's value stands.
    fn endpoint(&mut self, datum: Datum<'a>, leaf: &Base, at: &Location, reference: &RowRef) {
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
        self.write(leaf, value, at, reference, false);
    }

    /// Records one write, a fallback when `fallback` holds.
    ///
    /// No specification governs this: our own design. A row of the guide
    /// wins over the bridge's fallback for one element: a fallback at an
    /// element a write already fills is dropped, and a row of the guide
    /// drops every fallback at its element. Nothing the message carries is
    /// lost, since the fallback derives from the same value.
    fn write(
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
    ///
    /// # Errors
    ///
    /// Returns [`MapError::NoMessageHeader`] when the Bundle is a `message`
    /// and no complete `MessageHeader` enters it.
    pub(super) fn finish(mut self) -> Result<(Value, Vec<Outcome>), MapError> {
        let mut documents = Vec::new();
        let resources = core::mem::take(&mut self.resources);
        for resource in &resources {
            let document = self.build(resource);
            documents.push((resource, document));
        }
        let mut entries: Vec<(&Resource, Value)> = Vec::new();
        let mut envelope = None;
        for (resource, mut document) in documents {
            let complete = self.complete(resource, &mut document);
            if resource.identity == Identity::Envelope {
                envelope = Some(document);
                continue;
            }
            if !complete {
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
                    continue;
                }
            }
            entries.push((resource, document));
        }
        entries.sort_by_key(|(resource, _)| resource.type_name != "MessageHeader");
        let mut bundle = envelope.unwrap_or_else(|| document_of("Bundle"));
        let message = bundle.get("type").and_then(Value::as_str) == Some("message");
        let headed = entries
            .first()
            .is_some_and(|(resource, _)| resource.type_name == "MessageHeader");
        // NOTE: HL7 R4 Bundle invariant bdl-12: a `message` Bundle's first resource is a
        // MessageHeader, so a run that could not complete one refuses the message.
        if message && !headed {
            let dropped = self
                .outcomes
                .iter()
                .rev()
                .find(|outcome| match outcome {
                    Outcome::MissingRequired {
                        resource, element, ..
                    } => resource == "MessageHeader" && element == "MessageHeader",
                    Outcome::Undecodable { resource, .. } => resource == "MessageHeader",
                    _ => false,
                })
                .cloned()
                .map(Box::new);
            return Err(MapError::NoMessageHeader { dropped });
        }
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
        Ok((bundle, self.outcomes))
    }

    /// Drops from `document` every element that lacks one its definition
    /// requires, counting each, and answers whether the resource itself is
    /// complete.
    ///
    /// NOTE: no specification governs this: our own design; an element whose
    /// required sibling no row wrote is left out, so the resource decodes.
    fn complete(&mut self, resource: &Resource, document: &mut Value) -> bool {
        let (Value::Object(object), Some(schema)) =
            (document, SCHEMAS.type_named(&resource.type_name))
        else {
            return true;
        };
        let mut dropped = Vec::new();
        let incomplete = constraint::prune(object, schema, &resource.type_name, &mut dropped);
        let missing = |element: String, required: &str| Outcome::MissingRequired {
            resource: resource.type_name.clone(),
            full_url: resource.full_url.clone(),
            element,
            required: String::from(required),
        };
        for found in dropped {
            self.outcomes.push(missing(found.element, found.required));
        }
        let Some(required) = incomplete else {
            return true;
        };
        self.outcomes
            .push(missing(resource.type_name.clone(), required));
        false
    }

    /// Answers whether a write may go ahead, counting why when it may not: an
    /// element an earlier write filled, or a value outside the lexical form
    /// of the primitive `resolved` ends on.
    fn admits(
        &mut self,
        pending: &Write,
        resolved: Option<&fhirconnect::tree::element::Resolved>,
        value: &Value,
        superseded: bool,
    ) -> bool {
        if superseded {
            self.outcomes.push(Outcome::Superseded {
                at: pending.at.clone(),
                row: pending.row.clone(),
            });
            return false;
        }
        let Some((element, fhir_type, error)) =
            resolved.and_then(|resolved| refusal(resolved, value))
        else {
            return true;
        };
        self.outcomes.push(Outcome::InvalidValue {
            at: pending.at.clone(),
            row: pending.row.clone(),
            element,
            fhir_type: String::from(fhir_type),
            error,
        });
        false
    }

    /// Applies one resource's writes in order.
    fn build(&mut self, resource: &Resource) -> Value {
        let mut document = document_of(&resource.type_name);
        let mut indices: BTreeMap<(Vec<usize>, String, String), usize> = BTreeMap::new();
        let mut lengths: BTreeMap<(Vec<usize>, String), usize> = BTreeMap::new();
        let mut written: BTreeSet<(String, Vec<usize>)> = BTreeSet::new();
        let mut chosen: BTreeMap<(String, Vec<usize>), String> = BTreeMap::new();
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
            let path = format!("$resource{prefix}");
            let Ok(path) = path.parse::<FhirPath>() else {
                continue;
            };
            // NOTE: no specification governs this: our own design; a path that does not
            // resolve is left to `write`, which counts its refusal as `unwritable`.
            let resolved = resolve(&SCHEMAS, &resource.type_name, &path).ok();
            let leaf = (prefix.clone(), occurrence.clone());
            let taken = resolved
                .as_ref()
                .map(|resolved| alternatives(resolved, &occurrence))
                .unwrap_or_default();
            let superseded = written.contains(&leaf)
                || taken
                    .iter()
                    .any(|(choice, key)| chosen.get(choice).is_some_and(|earlier| earlier != key));
            if !self.admits(pending, resolved.as_ref(), &value, superseded) {
                continue;
            }
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
                    chosen.extend(taken);
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

/// The choice elements a write steps through, each keyed by its instance
/// (the members before it, the stem and the occurrence indices spent so far)
/// with the alternative the write takes.
///
/// A choice element that does not repeat holds one alternative
/// (<https://hl7.org/fhir/R4/json.html>, choice elements;
/// <https://hl7.org/fhir/R4/elementdefinition.html>, `ElementDefinition.max`),
/// so a write taking another alternative of an instance already written is
/// refused.
fn alternatives(
    resolved: &fhirconnect::tree::element::Resolved,
    occurrence: &[usize],
) -> Vec<((String, Vec<usize>), String)> {
    let mut found = Vec::new();
    let mut members = String::new();
    let mut spent = 0usize;
    for step in resolved.moves() {
        let (Move::Member(field) | Move::Extension { field, .. }) = step else {
            continue;
        };
        let stem = field
            .path()
            .rsplit('.')
            .next()
            .and_then(|name| name.strip_suffix("[x]"));
        if let Some(stem) = stem.filter(|_| !field.repeats()) {
            let spent_indices = occurrence.get(..spent).unwrap_or_default().to_vec();
            found.push((
                (format!("{members}.{stem}"), spent_indices),
                String::from(field.key()),
            ));
        }
        if field.repeats() {
            spent = spent.saturating_add(1);
        }
        members.push('.');
        members.push_str(field.key());
    }
    found
}

/// The element, primitive type and refusal of a value outside the lexical
/// form of the primitive `resolved` ends on, `None` when the value keeps it
/// or the path ends on no primitive.
fn refusal(
    resolved: &fhirconnect::tree::element::Resolved,
    value: &Value,
) -> Option<(String, &'static str, fhir_types::codec::DecodeErrorKind)> {
    if !matches!(
        resolved.location(),
        fhirconnect::tree::element::Location::Primitive(_)
            | fhirconnect::tree::element::Location::Attribute
    ) {
        return None;
    }
    let code = resolved.type_code().or_else(|| alternative(resolved))?;
    let error = constraint::lexical(code, value).err()?;
    Some((String::from(resolved.leaf()), code, error))
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

/// Whether `slots` lie at or under `prefix`, whose repeating steps are
/// `flags`; the key of a step that does not repeat names no instance.
fn under(prefix: &[Slot], flags: &[bool], slots: &[Slot]) -> bool {
    slots.len() >= prefix.len()
        && prefix
            .iter()
            .zip(slots)
            .zip(flags)
            .all(|((step, slot), repeats)| {
                step.name == slot.name && (!repeats || step.key == slot.key)
            })
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

/// The data type the placed segment's definition gives a field, when its
/// `TypeInfo` names none: the definition of the version the parser selected
/// the structure from, so a legacy field keeps its own version's type.
///
/// The guide's maps come first: a legacy code the guide has a data type map
/// for keeps that map (`TS` keeps `datatype-ts-to-datetime`), and one it has
/// none for gives the base type the generated tables link it to
/// ([`version_specific`]): `CM_MSG` gives `MSG` and `CE_0051` gives `CE`.
fn definition_type(
    corpus: &Corpus,
    definition: &'static hl7v2_types::model::Segment,
    position: usize,
) -> Option<String> {
    if let Some((_, base)) = version_specific(corpus, definition, position) {
        return Some(String::from(base.code));
    }
    definition
        .fields
        .iter()
        .find(|field| usize::from(field.position) == position)?
        .data_type
        .map(|data_type| String::from(data_type.code()))
}

/// The legacy data type code of a field that stands for a base type, with
/// that base, as the generated tables link them, when the guide has no data
/// type map from the code as written.
fn version_specific(
    corpus: &Corpus,
    definition: &'static hl7v2_types::model::Segment,
    position: usize,
) -> Option<(
    &'static hl7v2_types::model::LegacyDataType,
    hl7v2_types::model::LegacyBase,
)> {
    let field = definition
        .fields
        .iter()
        .find(|field| usize::from(field.position) == position)?;
    match field.data_type {
        Some(hl7v2_types::model::DataTypeRef::Legacy(code))
            if !corpus.maps_from("datatype", code.code) =>
        {
            Some((code, code.base?))
        }
        _ => None,
    }
}

/// The data type a field row maps its value by, with the type the row names
/// when the field's own definition replaces it.
///
/// The row's `TypeInfo` type comes first. For a field of a legacy structure
/// (`legacy`), the placed definition's type decides where it differs from
/// the row's, compared without case, since the guide's rows name the types
/// of the v2.9.1 definitions. No specification governs this: our own design,
/// `mapping_guidelines.md` names no v2 version for its type columns.
fn field_type(
    named: Option<String>,
    own: Option<String>,
    legacy: bool,
) -> (Option<String>, Option<String>) {
    match (named, own) {
        (Some(named), Some(own)) if legacy && !named.eq_ignore_ascii_case(&own) => {
            (Some(own), Some(named))
        }
        (named, own) => (named.or(own), None),
    }
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

/// Groups `maps` into sets joined by `related`, each set by id.
fn group<'m>(maps: Vec<&'m Map>, related: fn(&Map, &Map) -> bool) -> Vec<Vec<&'m Map>> {
    let mut sets: Vec<Vec<&'m Map>> = Vec::new();
    for map in maps {
        let (joined, mut apart): (Vec<_>, Vec<_>) = std::mem::take(&mut sets)
            .into_iter()
            .partition(|set| set.iter().any(|other| related(other, map)));
        let mut merged: Vec<&'m Map> = joined.into_iter().flatten().collect();
        merged.push(map);
        merged.sort_by(|a, b| a.id.cmp(&b.id));
        apart.push(merged);
        sets = apart;
    }
    sets
}

/// A target code with every `[..]` label removed, `identifier[2]` giving
/// `identifier`.
fn unlabelled(code: &str) -> String {
    let mut text = String::with_capacity(code.len());
    let mut depth = 0_usize;
    for character in code.chars() {
        match character {
            '[' => depth = depth.saturating_add(1),
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(character),
            _ => {}
        }
    }
    text
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
    definition: &'static hl7v2_types::model::Segment,
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
                    definition: placed.node.segment,
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

/// The path of every segment node of a structure's tree, as a message map
/// row names it: `ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR`.
fn tree_paths(structure: &str, nodes: &'static [hl7v2_types::model::Node]) -> BTreeSet<String> {
    fn walk(prefix: &str, nodes: &'static [hl7v2_types::model::Node], out: &mut BTreeSet<String>) {
        for node in nodes {
            match node {
                hl7v2_types::model::Node::Segment(reference) => {
                    out.insert(format!("{prefix}.{}", reference.segment.id));
                }
                hl7v2_types::model::Node::Group(group) => {
                    walk(&format!("{prefix}.{}", group.name), group.children, out);
                }
                hl7v2_types::model::Node::Placeholder(_) => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(structure, nodes, &mut out);
    out
}

/// The segment ids under each group of a structure's tree, directly or in
/// its child groups, keyed by the group's path (`ORM_O01.PATIENT`).
fn tree_contents(
    structure: &str,
    nodes: &'static [hl7v2_types::model::Node],
) -> BTreeMap<String, BTreeSet<&'static str>> {
    fn walk(
        prefix: &str,
        nodes: &'static [hl7v2_types::model::Node],
        out: &mut BTreeMap<String, BTreeSet<&'static str>>,
    ) -> BTreeSet<&'static str> {
        let mut held = BTreeSet::new();
        for node in nodes {
            match node {
                hl7v2_types::model::Node::Segment(reference) => {
                    held.insert(reference.segment.id);
                }
                hl7v2_types::model::Node::Group(group) => {
                    let path = format!("{prefix}.{}", group.name);
                    let inner = walk(&path, group.children, out);
                    held.extend(inner.iter().copied());
                    out.entry(path).or_default().extend(inner);
                }
                hl7v2_types::model::Node::Placeholder(_) => {}
            }
        }
        held
    }
    let mut out = BTreeMap::new();
    walk(structure, nodes, &mut out);
    out
}

/// The segment ids a message map's rows place under each group path of
/// their sources, keyed as [`tree_contents`] keys a tree's.
fn row_contents(map: &Map) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in &map.rows {
        if row.source.contains(":follow:") {
            continue;
        }
        let Some((groups, segment)) = row.source.rsplit_once('.') else {
            continue;
        };
        let mut prefix = groups;
        while let Some((parent, _)) = prefix.rsplit_once('.') {
            out.entry(String::from(prefix))
                .or_default()
                .insert(String::from(segment));
            prefix = parent;
        }
    }
    out
}

/// The segments under each group, of the guide's rows and of the tree.
#[derive(Debug, Default)]
struct Contents {
    rows: BTreeMap<String, BTreeSet<String>>,
    tree: BTreeMap<String, BTreeSet<&'static str>>,
}

impl Contents {
    /// Whether the tree group at `tree` holds every segment the rows place in
    /// the row group at `row`, one of them at least.
    fn holds(&self, row: &str, tree: &str) -> bool {
        match (self.rows.get(row), self.tree.get(tree)) {
            (Some(placed), Some(held)) => {
                !placed.is_empty() && placed.iter().all(|id| held.contains(id.as_str()))
            }
            _ => false,
        }
    }
}

/// How a row source's groups pair with a tree path's.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pairing {
    /// Whether one group is added or omitted.
    shifted: bool,
    /// Each row group paired with a tree group of another name, as `(row,
    /// tree)` in path order.
    renamed: Vec<(String, String)>,
}

/// A row source reaching a parsed segment's path, with how its groups pair.
#[derive(Debug, PartialEq, Eq)]
struct Regrouping<'m> {
    source: &'m str,
    shifted: bool,
    renamed: Vec<(String, String)>,
}

/// The one message map row source that reaches the parsed segment's `path`
/// among the `tree` paths of its structure.
///
/// The guide's message maps name the group paths of one structure per
/// message (`ORM_O01.ORDER_DETAIL.CHOICE.OBR`, `ORM_O01.PATIENT.VISIT.PV1`),
/// while a legacy tree nests and names the groups of its own version
/// (`ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR` and
/// `ORM_O01.PATIENT.PATIENT_VISIT.PV1` at 2.3). A row source reaches a path
/// when [`pairing`] pairs their groups, and the pairing with the fewest
/// renamed groups counts. A row source any tree path names exactly belongs
/// to that path. The match holds only when one row source reaches `path` at
/// the fewest renames and that source reaches no other tree path at as few;
/// otherwise the segment stays unmapped. The rule holds for every tree,
/// legacy and v2.9.1 alike, since the guide's paths also disagree with the
/// v2.9.1 definitions (`OML_O21`). No specification governs this: our own
/// design, since `mapping_guidelines.md` writes one structure per message.
fn regrouped<'m>(
    map: &'m Map,
    path: &str,
    tree: &BTreeSet<String>,
    contents: &Contents,
) -> Option<Regrouping<'m>> {
    let sources: BTreeSet<&str> = map
        .rows
        .iter()
        .map(|row| row.source.as_str())
        .filter(|source| !source.contains(":follow:") && !tree.contains(*source))
        .collect();
    let (source, found) = cheapest(
        sources
            .into_iter()
            .filter_map(|source| pairing(source, path, contents).map(|found| (source, found))),
    )?;
    let (only, _) = cheapest(
        tree.iter()
            .filter_map(|other| pairing(source, other, contents).map(|found| (other, found))),
    )?;
    (only == path).then_some(Regrouping {
        source,
        shifted: found.shifted,
        renamed: found.renamed,
    })
}

/// The one candidate with the fewest renamed groups; `None` when there is
/// none or two share the fewest.
fn cheapest<T>(candidates: impl Iterator<Item = (T, Pairing)>) -> Option<(T, Pairing)> {
    let candidates: Vec<(T, Pairing)> = candidates.collect();
    let least = candidates
        .iter()
        .map(|(_, found)| found.renamed.len())
        .min()?;
    let mut best = candidates
        .into_iter()
        .filter(|(_, found)| found.renamed.len() == least);
    let first = best.next()?;
    best.next().is_none().then_some(first)
}

/// The path of a group: the structure and the groups up to `index`.
fn group_path(structure: &str, groups: &[&str], index: usize) -> String {
    let mut out = String::from(structure);
    for group in groups.iter().take(index.saturating_add(1)) {
        out.push('.');
        out.push_str(group);
    }
    out
}

/// The ways to pair `rows` groups with `trees` groups in order, as index
/// pairs, when one side holds at most one group more and that group is not
/// the last.
fn alignments(rows: usize, trees: usize) -> Vec<Vec<(usize, usize)>> {
    let skip = |index: usize, gap: usize| {
        if index < gap {
            index
        } else {
            index.saturating_add(1)
        }
    };
    if rows == trees {
        vec![(0..rows).map(|index| (index, index)).collect()]
    } else if trees == rows.saturating_add(1) {
        (0..rows)
            .map(|gap| (0..rows).map(|index| (index, skip(index, gap))).collect())
            .collect()
    } else if rows == trees.saturating_add(1) {
        (0..trees)
            .map(|gap| (0..trees).map(|index| (skip(index, gap), index)).collect())
            .collect()
    } else {
        Vec::new()
    }
}

/// How the groups of a row `source` pair with those of a tree `path`, when
/// they name one structure and one segment.
///
/// The groups pair in order, one of the two paths holding at most one group
/// more, which is not the group holding the segment. A pair names one group,
/// or two names when the tree group holds every segment the guide's rows
/// place in the row group ([`Contents::holds`]), which pairs the guide's
/// `ORM_O01.PATIENT.VISIT` with the 2.3 `ORM_O01.PATIENT.PATIENT_VISIT`. Of
/// the ways to pair, the one with the fewest renamed groups counts, and two
/// that differ leave the paths unpaired.
fn pairing(source: &str, path: &str, contents: &Contents) -> Option<Pairing> {
    let row: Vec<&str> = source.split('.').collect();
    let tree: Vec<&str> = path.split('.').collect();
    let ((row_structure, row_rest), (tree_structure, tree_rest)) =
        (row.split_first()?, tree.split_first()?);
    let ((row_segment, row_groups), (tree_segment, tree_groups)) =
        (row_rest.split_last()?, tree_rest.split_last()?);
    if row_structure != tree_structure || row_segment != tree_segment {
        return None;
    }
    let shifted = row_groups.len() != tree_groups.len();
    let mut found: Vec<Pairing> = Vec::new();
    for alignment in alignments(row_groups.len(), tree_groups.len()) {
        let mut renamed = Vec::new();
        let paired = alignment.iter().all(|&(at_row, at_tree)| {
            let (Some(named), Some(held)) = (row_groups.get(at_row), tree_groups.get(at_tree))
            else {
                return false;
            };
            if named == held {
                return true;
            }
            let holds = contents.holds(
                &group_path(row_structure, row_groups, at_row),
                &group_path(tree_structure, tree_groups, at_tree),
            );
            if holds {
                renamed.push((String::from(*named), String::from(*held)));
            }
            holds
        });
        let candidate = Pairing { shifted, renamed };
        if paired && !found.contains(&candidate) {
            found.push(candidate);
        }
    }
    cheapest(found.into_iter().map(|found| ((), found))).map(|((), found)| found)
}

/// Whether a node repeats.
const fn repeats(max: Max) -> bool {
    match max {
        Max::Unbounded => true,
        Max::Bounded(bound) => bound > 1,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fhir_types::codec::{Json, Path, Value};

    use super::{
        Contents, Outcome, Pending, Run, alternatives, collect, definition_type, field_type,
        pairing, regrouped, resolve_path, row_contents, tree_contents, tree_paths,
    };
    use crate::decode::Charset;
    use crate::map::corpus::Corpus;
    use crate::parse::{self, Parsed};

    /// A synthetic result whose MSH carries `header` as MSH-3 to MSH-6.
    fn parsed(header: &str) -> Parsed {
        parsed_with(header, "")
    }

    /// A synthetic result as [`parsed`], with `tail` after MSH-12.
    fn parsed_with(header: &str, tail: &str) -> Parsed {
        let text = format!(
            "MSH|^~\\&|{header}|20260925143000+0200||ORU^R01^ORU_R01|MSG00011|P|2.5.1{tail}\r\
             PID|1||PAT-0011^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M\r\
             OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN\r"
        );
        let lexed = parse::lex(&text, Charset::Ascii).expect("it lexes");
        let structure = parse::structure_for(&lexed.message).expect("a structure");
        parse::group(lexed, structure)
    }

    fn corpus() -> Corpus {
        let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/package");
        Corpus::load(&package).expect("the vendored package loads")
    }

    /// The `MessageHeader` writes of a run, as dotted paths and ready values.
    fn header_writes(run: &Run<'_>) -> Vec<(String, Value)> {
        writes_of(run, "MessageHeader")
    }

    /// The writes of a run into resources of `type_name`, as dotted paths and
    /// ready values.
    fn writes_of(run: &Run<'_>, type_name: &str) -> Vec<(String, Value)> {
        run.resources
            .iter()
            .filter(|resource| resource.type_name == type_name)
            .flat_map(|resource| &resource.writes)
            .filter_map(|write| {
                let path: Vec<&str> = write.slots.iter().map(|slot| slot.name.as_str()).collect();
                match &write.value {
                    Pending::Ready(value) => Some((path.join("."), value.clone())),
                    Pending::Translation { .. } => None,
                }
            })
            .collect()
    }

    // NOTE: `segment-msh-to-messageheader`: the MSH-24 rows gated on
    // `IF MSH-24 NOT VALUED AND MSH-3 NOT VALUED` assign the data-absent-reason.
    #[test]
    fn rows_gated_on_their_own_empty_source_write_their_assignment() {
        let corpus = corpus();
        let parsed = parsed("||EHR|SOUTHCLINIC");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let writes = header_writes(&run);
        let reason = String::from("http://hl7.org/fhir/R4/extension-data-absent-reason.html");
        assert!(
            writes.contains(&(
                String::from("source.endpoint.extension.url"),
                Value::String(reason)
            )),
            "{writes:?}"
        );
        assert!(
            writes.contains(&(
                String::from("source.endpoint.extension.valueCode"),
                Value::String(String::from("unknown"))
            )),
            "{writes:?}"
        );
    }

    // NOTE: the HL7 v2.3 tables type OBR-4 `CE` where the guide's row names `CWE`, so the
    // placed 2.3 definition chooses the guide's `CE` map, counted as `version-typed`.
    #[test]
    fn a_legacy_field_is_typed_by_its_own_versions_definition() {
        let text = "MSH|^~\\&|NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC|20260925143000+0200||ORM^O01|MSG00012|P|2.3\r\
             PID|1||PAT-0012^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M\r\
             ORC|NW|PLC-2\r\
             OBR|1|PLC-2||2345-7^Glucose^LN\r";
        let lexed = parse::lex(text, Charset::Ascii).expect("it lexes");
        let structure = parse::structure_for(&lexed.message).expect("a structure");
        assert_eq!((structure.id, structure.version), ("ORM_O01", "2.3"));
        let parsed = parse::group(lexed, structure);
        let mut visits = Vec::new();
        collect(
            parsed.items(),
            parsed.structure_name(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            parsed.structure().nodes,
            &mut visits,
        );
        let obr = visits
            .iter()
            .find(|visit| visit.code == "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR")
            .expect("the OBR is placed");
        let legacy = definition_type(&corpus(), obr.definition, 4).expect("OBR-4 is typed at 2.3");
        assert_eq!(legacy, "CE");
        assert_eq!(
            definition_type(&corpus(), &hl7v2_types::segment::obr::OBR, 4).as_deref(),
            Some("CWE")
        );
        let corpus = corpus();
        let chosen = corpus
            .find("datatype", &legacy, "CodeableConcept")
            .expect("one map");
        assert_eq!(chosen.id, "datatype-ce-to-codeableconcept");
        assert_eq!(
            field_type(Some(String::from("CWE")), Some(legacy.clone()), true),
            (Some(legacy), Some(String::from("CWE")))
        );
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let typed: Vec<(&str, &str, &str, &str)> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::VersionTyped {
                    field,
                    row_type,
                    version_type,
                    version,
                    ..
                } => Some((
                    field.as_str(),
                    row_type.as_str(),
                    version_type.as_str(),
                    *version,
                )),
                _ => None,
            })
            .collect();
        assert!(typed.contains(&("PID-8", "CWE", "IS", "2.3")), "{typed:?}");
    }

    /// A synthetic 2.3 ORM^O01 with a laboratory order carrying a note, a
    /// diagnosis and a result, and a pharmacy order.
    fn legacy_order() -> Parsed {
        message(&[
            "MSH|^~\\&|NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC|20260925143000+0200||ORM^O01|MSG00031|P|2.3",
            "PID|1||PAT-0031^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
            "ORC|NW|PLC-31",
            "OBR|1|PLC-31||2345-7^Glucose^LN",
            "NTE|1||Fasting sample",
            "DG1|1||E11.9^Type 2 diabetes^I10",
            "OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L",
            "ORC|NW|PLC-32",
            "RXO|RX-1^Metformin^NDC|500",
        ])
    }

    #[test]
    fn a_legacy_order_detail_reaches_the_guides_rows_through_its_group_path() {
        let parsed = legacy_order();
        assert_eq!(parsed.structure().version, "2.3");
        let corpus = corpus();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let matched: Vec<(&str, &str)> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::GroupPath {
                    row_path,
                    tree_path,
                    ..
                } => Some((row_path.as_str(), tree_path.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            matched,
            [
                (
                    "ORM_O01.ORDER_DETAIL.CHOICE.OBR",
                    "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR"
                ),
                ("ORM_O01.ORDER_DETAIL.NTE", "ORM_O01.ORDER.ORDER_DETAIL.NTE"),
                ("ORM_O01.ORDER_DETAIL.DG1", "ORM_O01.ORDER.ORDER_DETAIL.DG1"),
                (
                    "ORM_O01.ORDER_DETAIL.OBSERVATION.OBX",
                    "ORM_O01.ORDER.ORDER_DETAIL.OBSERVATION.OBX"
                ),
                (
                    "ORM_O01.ORDER_DETAIL.CHOICE.RXO",
                    "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.RXO"
                ),
            ]
        );
        let unmapped: Vec<&Outcome> = run
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::UnmappedSegment { .. }))
            .collect();
        assert!(unmapped.is_empty(), "{unmapped:?}");
        for type_name in ["ServiceRequest", "Observation", "MedicationRequest"] {
            assert!(
                !writes_of(&run, type_name).is_empty(),
                "the order detail writes a {type_name}"
            );
        }
    }

    #[test]
    fn a_group_path_that_reaches_two_tree_paths_stays_unmapped() {
        let corpus = corpus();
        let map = corpus
            .message_map("ORM_O01")
            .expect("the ORM_O01 message map");
        let obr = "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR";
        let none = Contents::default();
        let one: std::collections::BTreeSet<String> = [String::from(obr)].into();
        assert_eq!(
            regrouped(map, obr, &one, &none).map(|found| found.source),
            Some("ORM_O01.ORDER_DETAIL.CHOICE.OBR")
        );
        let two: std::collections::BTreeSet<String> = [
            String::from(obr),
            String::from("ORM_O01.ORDER_DETAIL.EXTRA.CHOICE.OBR"),
        ]
        .into();
        assert_eq!(regrouped(map, obr, &two, &none), None);
        let top: std::collections::BTreeSet<String> = [String::from("ORM_O01.NTE")].into();
        assert_eq!(regrouped(map, "ORM_O01.NTE", &top, &none), None);
    }

    /// Whether two paths pair by one group added or omitted, with no group
    /// renamed.
    fn one_group_apart(left: &str, right: &str) -> bool {
        pairing(left, right, &Contents::default())
            .is_some_and(|found| found.shifted && found.renamed.is_empty())
    }

    /// A synthetic 2.3 ORM^O01 whose patient carries a visit.
    fn legacy_visit() -> Parsed {
        message(&[
            "MSH|^~\\&|NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC|20260925143000+0200||ORM^O01|MSG00032|P|2.3",
            "PID|1||PAT-0032^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
            "PV1|1|I|WARD-3^ROOM-12^BED-1",
            "PV2|||^Observation",
            "ORC|NW|PLC-33",
            "OBR|1|PLC-33||2345-7^Glucose^LN",
        ])
    }

    /// The `group-renamed` outcomes of a run, as row path, tree path, row
    /// group and tree group.
    fn renamed(run: &Run<'_>) -> Vec<(String, String, String, String)> {
        run.outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::GroupRenamed {
                    row_path,
                    tree_path,
                    row_group,
                    tree_group,
                    ..
                } => Some((
                    row_path.clone(),
                    tree_path.clone(),
                    row_group.clone(),
                    tree_group.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_legacy_patient_visit_reaches_the_guides_visit_rows_by_its_segments() {
        let parsed = legacy_visit();
        assert_eq!(parsed.structure().version, "2.3");
        let corpus = corpus();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let pair = |segment: &str| {
            (
                format!("ORM_O01.PATIENT.VISIT.{segment}"),
                format!("ORM_O01.PATIENT.PATIENT_VISIT.{segment}"),
                String::from("VISIT"),
                String::from("PATIENT_VISIT"),
            )
        };
        assert_eq!(renamed(&run), [pair("PV1"), pair("PV2")]);
        let unmapped: Vec<&Outcome> = run
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::UnmappedSegment { .. }))
            .collect();
        assert!(unmapped.is_empty(), "{unmapped:?}");
        assert!(
            !writes_of(&run, "Encounter").is_empty(),
            "the visit writes an Encounter"
        );
    }

    #[test]
    fn a_renamed_and_a_shifted_group_in_one_path_pair_uniquely() {
        let corpus = corpus();
        let map = corpus
            .message_map("ORM_O01")
            .expect("the ORM_O01 message map");
        let pv1 = "ORM_O01.PATIENT.EXTRA.PATIENT_VISIT.PV1";
        let tree: std::collections::BTreeSet<String> = [
            String::from(pv1),
            String::from("ORM_O01.PATIENT.EXTRA.PATIENT_VISIT.PV2"),
        ]
        .into();
        let held: std::collections::BTreeSet<&'static str> = ["PV1", "PV2"].into();
        let contents = Contents {
            rows: row_contents(map),
            tree: [
                (String::from("ORM_O01.PATIENT"), held.clone()),
                (String::from("ORM_O01.PATIENT.EXTRA"), held.clone()),
                (String::from("ORM_O01.PATIENT.EXTRA.PATIENT_VISIT"), held),
            ]
            .into(),
        };
        let found = regrouped(map, pv1, &tree, &contents).expect("the visit rows reach PV1");
        assert_eq!(found.source, "ORM_O01.PATIENT.VISIT.PV1");
        assert!(found.shifted);
        assert_eq!(
            found.renamed,
            [(String::from("VISIT"), String::from("PATIENT_VISIT"))]
        );
    }

    #[test]
    fn two_tree_groups_holding_the_guides_segments_leave_the_segment_unmapped() {
        let corpus = corpus();
        let map = corpus
            .message_map("ORM_O01")
            .expect("the ORM_O01 message map");
        let first = "ORM_O01.PATIENT.PATIENT_VISIT.PV1";
        let tree: std::collections::BTreeSet<String> = [
            String::from(first),
            String::from("ORM_O01.PATIENT.PATIENT_VISIT.PV2"),
            String::from("ORM_O01.PATIENT.OTHER_VISIT.PV1"),
            String::from("ORM_O01.PATIENT.OTHER_VISIT.PV2"),
        ]
        .into();
        let held: std::collections::BTreeSet<&'static str> = ["PV1", "PV2"].into();
        let contents = Contents {
            rows: row_contents(map),
            tree: [
                (String::from("ORM_O01.PATIENT.PATIENT_VISIT"), held.clone()),
                (String::from("ORM_O01.PATIENT.OTHER_VISIT"), held),
            ]
            .into(),
        };
        assert_eq!(regrouped(map, first, &tree, &contents), None);
        let lacking = Contents {
            rows: row_contents(map),
            tree: [(
                String::from("ORM_O01.PATIENT.PATIENT_VISIT"),
                ["PV1"].into(),
            )]
            .into(),
        };
        let only: std::collections::BTreeSet<String> = [String::from(first)].into();
        assert_eq!(
            regrouped(map, first, &only, &lacking),
            None,
            "a tree group lacking the guide's PV2 is no rename"
        );
    }

    #[test]
    fn a_current_result_renames_no_group() {
        let parsed = message(&[
            "MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00033|P|2.5.1",
            "PID|1||PAT-0033^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
            "PV1|1|O",
            "ORC|RE|PLC-1|FIL-1",
            "OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
            "NTE|1||Fasting sample",
            "OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L",
            "SPM|1|SPC-1||BLD^Blood^HL70487",
        ]);
        assert_eq!(parsed.structure().version, "2.9.1");
        let corpus = corpus();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert!(renamed(&run).is_empty(), "{:?}", renamed(&run));
    }

    #[test]
    fn the_tree_contents_hold_the_segments_of_each_group() {
        let structure = hl7v2_types::legacy::find("ORM_O01", "2.3").expect("ORM_O01 at 2.3");
        let contents = tree_contents("ORM_O01", structure.nodes);
        let visit = contents
            .get("ORM_O01.PATIENT.PATIENT_VISIT")
            .expect("the 2.3 visit group");
        assert_eq!(visit.iter().copied().collect::<Vec<_>>(), ["PV1", "PV2"]);
        let patient = contents.get("ORM_O01.PATIENT").expect("the patient group");
        assert!(patient.contains("PID") && patient.contains("PV1"));
    }

    #[test]
    fn two_paths_one_group_apart_share_the_group_holding_the_segment() {
        let row = "ORM_O01.ORDER_DETAIL.CHOICE.OBR";
        assert!(one_group_apart(
            row,
            "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR"
        ));
        assert!(one_group_apart(row, "ORM_O01.CHOICE.OBR"));
        assert!(!one_group_apart(row, "ORM_O01.ORDER_DETAIL.OBR"));
        assert!(!one_group_apart(row, "ORM_O01.ORDER_DETAIL.CHOICE.X.OBR"));
        assert!(!one_group_apart(row, "ORM_O01.A.B.ORDER_DETAIL.CHOICE.OBR"));
        assert!(!one_group_apart(
            row,
            "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.RXO"
        ));
        assert!(!one_group_apart(
            row,
            "ORU_R01.ORDER.ORDER_DETAIL.CHOICE.OBR"
        ));
        assert!(!one_group_apart("ORM_O01.ORDER_DETAIL.NTE", "ORM_O01.NTE"));
    }

    #[test]
    fn the_tree_paths_name_every_segment_node_by_its_groups() {
        let structure = hl7v2_types::legacy::find("ORM_O01", "2.3").expect("ORM_O01 at 2.3");
        let paths = tree_paths("ORM_O01", structure.nodes);
        assert!(paths.contains("ORM_O01.MSH"));
        assert!(paths.contains("ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR"));
        assert!(paths.contains("ORM_O01.ORDER.ORDER_DETAIL.OBSERVATION.OBX"));
    }

    #[test]
    fn a_version_specific_field_type_resolves_to_its_base_for_the_map_lookup() {
        let msh = &hl7v2_types::legacy::v2_3::segment::msh::MSH;
        let dg1 = &hl7v2_types::legacy::v2_3::segment::dg1::DG1;
        let corpus = corpus();
        assert_eq!(definition_type(&corpus, msh, 9).as_deref(), Some("MSG"));
        assert_eq!(definition_type(&corpus, dg1, 3).as_deref(), Some("CE"));
        assert_eq!(definition_type(&corpus, msh, 4).as_deref(), Some("HD"));
        let parsed = legacy_order();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let based: Vec<(&str, &str, &str, &str)> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::BaseTyped {
                    field,
                    code,
                    base,
                    version,
                    ..
                } => Some((field.as_str(), *code, *base, *version)),
                _ => None,
            })
            .collect();
        for expected in [
            ("MSH-9", "CM_MSG", "MSG", "2.3"),
            ("DG1-3", "CE_0051", "CE", "2.3"),
        ] {
            assert!(based.contains(&expected), "{expected:?} in {based:?}");
        }
        assert!(
            based.iter().all(|(_, code, _, _)| *code != "TS"),
            "{based:?}"
        );
        let unmapped_types: Vec<&Outcome> = run
            .outcomes
            .iter()
            .filter(|outcome| {
                matches!(outcome, Outcome::NoDatatypeMap { source_type, .. }
                    if ["CM_MSG", "CE_0051", "TS"].contains(&source_type.as_str()))
            })
            .collect();
        assert!(unmapped_types.is_empty(), "{unmapped_types:?}");
    }

    #[test]
    fn a_legacy_code_the_guide_maps_keeps_the_guides_map() {
        let corpus = corpus();
        let msh = &hl7v2_types::legacy::v2_3::segment::msh::MSH;
        assert!(corpus.maps_from("datatype", "TS"));
        assert!(!corpus.maps_from("datatype", "CM_MSG"));
        assert_eq!(definition_type(&corpus, msh, 7).as_deref(), Some("TS"));
        let chosen = corpus
            .find("datatype", "TS", "dateTime")
            .expect("the guide's TS map");
        assert_eq!(chosen.id, "datatype-ts-to-datetime");
    }

    // NOTE: the guide's ORC-4 rows name `EIP` and the v2.9.1 definitions `EI`; a
    // v2.9.1 structure keeps the row's type and counts no substitution.
    #[test]
    fn a_current_field_keeps_the_type_its_row_names() {
        let text = "MSH|^~\\&|NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00013|P|2.5.1\r\
             PID|1||PAT-0013^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M\r\
             ORC|RE|PLC-3|FIL-3|GRP-3^NORTHLAB\r\
             OBR|1|PLC-3|FIL-3|2345-7^Glucose^LN\r";
        let lexed = parse::lex(text, Charset::Ascii).expect("it lexes");
        let structure = parse::structure_for(&lexed.message).expect("a structure");
        assert_eq!(structure.withdrawn_as_of, None);
        let parsed = parse::group(lexed, structure);
        assert_eq!(
            definition_type(&corpus(), &hl7v2_types::segment::orc::ORC, 4).as_deref(),
            Some("EI")
        );
        assert_eq!(
            field_type(Some(String::from("EIP")), Some(String::from("EI")), false),
            (Some(String::from("EIP")), None)
        );
        let corpus = corpus();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let substituted = run
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::VersionTyped { .. }))
            .count();
        assert_eq!(substituted, 0, "{:?}", run.outcomes);
    }

    /// MSH-24 alone valued, as `NORTHNET^1.2.3.9^ISO`.
    const NETWORK_ONLY: (&str, &str) = (
        "|NORTHLAB|EHR|SOUTHCLINIC",
        "||||||||||||NORTHNET^1.2.3.9^ISO",
    );

    /// A data type map with one row per `(component, child, type)` triple, at
    /// the canonical url `url`.
    fn datatype_map(id: &str, url: &str, rows: &[(&str, &str, &str)]) -> Value {
        let elements: Vec<serde_json::Value> = rows
            .iter()
            .map(|(component, child, fhir_type)| {
                serde_json::json!({
                    "code": component,
                    "target": [{
                        "code": child,
                        "equivalence": "equivalent",
                        "extension": [{
                            "url": crate::map::corpus::TYPE_INFO,
                            "extension": [{ "url": "type", "valueCode": fhir_type }]
                        }]
                    }]
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "resourceType": "ConceptMap",
            "id": id,
            "url": url,
            "status": "active",
            "group": [{ "element": elements }]
        }))
        .expect("a JSON value")
    }

    /// The vendored package with `maps` loaded over it as a supplement.
    fn supplemented(maps: &[Value]) -> Corpus {
        let directory = tempfile::tempdir().expect("a temporary directory");
        for (index, map) in maps.iter().enumerate() {
            let path = directory.path().join(format!("ConceptMap-{index}.json"));
            std::fs::write(path, serde_json::to_vec(map).expect("it serializes")).expect("written");
        }
        corpus()
            .supplement(directory.path())
            .expect("the supplement loads")
    }

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

    #[test]
    fn a_valued_source_takes_no_absence_row() {
        let corpus = corpus();
        let parsed = parsed("LAB|NORTHLAB|EHR|SOUTHCLINIC");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let writes = header_writes(&run);
        assert!(
            writes.iter().all(|(path, _)| !path.contains("extension")),
            "{writes:?}"
        );
    }

    /// A synthetic message of `segments`, each ended by a carriage return.
    fn message(segments: &[&str]) -> Parsed {
        let text = format!("{}\r", segments.join("\r"));
        let lexed = parse::lex(&text, Charset::Ascii).expect("it lexes");
        let structure = parse::structure_for(&lexed.message).expect("a structure");
        parse::group(lexed, structure)
    }

    /// A synthetic ORU^R01 whose one order carries `orc`.
    fn order(orc: &str) -> Parsed {
        message(&[
            "MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00021|P|2.5.1",
            "PID|1||PAT-0021^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
            orc,
            "OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
        ])
    }

    /// The values written at `path` in resources of `type_name`.
    fn values_at(run: &Run<'_>, type_name: &str, path: &str) -> Vec<Value> {
        writes_of(run, type_name)
            .into_iter()
            .filter(|(written, _)| written == path)
            .map(|(_, value)| value)
            .collect()
    }

    fn text(value: &str) -> Value {
        Value::String(String::from(value))
    }

    /// The conflicts, ambiguities and missing `ST` maps a run counted, the
    /// refusals an EI value can meet.
    fn ei_refusals<'r>(run: &'r Run<'_>) -> Vec<&'r Outcome> {
        run.outcomes
            .iter()
            .filter(|outcome| match outcome {
                Outcome::DatatypeConflict { .. } | Outcome::DatatypeAmbiguous { .. } => true,
                Outcome::NoDatatypeMap { row, .. } => row.map == "datatype-st-to-identifier",
                _ => false,
            })
            .collect()
    }

    // NOTE: `ConceptMap-datatype-ei-*-to-identifier`: the four variants each write `EI.1` into
    // `value`, and `hd-endpoint` writes `name` from HD.1 only under a condition.
    #[test]
    fn the_ei_variants_are_alternatives_and_the_hd_maps_into_one_source_are_not() {
        let corpus = corpus();
        let map = |id: &str| corpus.get(id).expect("a vendored map");
        let organization = map("datatype-ei-organization-to-identifier");
        for other in [
            "datatype-ei-defaultassigner-to-identifier",
            "datatype-ei-extension-to-identifier",
            "datatype-ei-system-to-identifier",
        ] {
            assert!(organization.alternative_to(map(other)), "{other}");
        }
        assert!(
            !map("datatype-hd-endpoint-to-messageheader-source")
                .alternative_to(map("datatype-hd-name-to-messageheader-source"))
        );
    }

    // NOTE: `segment-orc-to-diagnosticreport` ORC-2 and ORC-3 into `identifier[1]` and `[2]`; a
    // bare EI takes the plain variant, `datatype-ei-defaultassigner-to-identifier`, once.
    #[test]
    fn a_bare_placer_and_filler_number_give_one_identifier_each() {
        let corpus = corpus();
        let parsed = order("ORC|RE|PLC-1|FIL-1");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("PLC-1"), text("FIL-1")],
            "{:?}",
            writes_of(&run, "DiagnosticReport")
        );
        assert!(
            writes_of(&run, "DiagnosticReport")
                .iter()
                .all(|(path, _)| path != "identifier.system" && path != "identifier.assigner"),
            "{:?}",
            writes_of(&run, "DiagnosticReport")
        );
        assert_eq!(ei_refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: `datatype-ei-organization-to-identifier` maps EI.2 into the assigner, the one variant
    // with a row that runs on a namespace ID, so it is the most specific.
    #[test]
    fn a_value_with_an_assigning_authority_takes_the_organization_variant() {
        let corpus = corpus();
        let parsed = order("ORC|RE|PLC-1^ORDERDESK|FIL-1");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("PLC-1"), text("FIL-1")]
        );
        assert!(
            values_at(&run, "Organization", "identifier.value").contains(&text("ORDERDESK")),
            "{:?}",
            writes_of(&run, "Organization")
        );
        assert_eq!(ei_refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: `segment-txa-to-documentreference` TXA-12 into `masterIdentifier`.
    #[test]
    fn a_unique_document_number_gives_one_master_identifier() {
        let corpus = corpus();
        let parsed = message(&[
            "MSH|^~\\&|DOCS|NORTHHOSP|EHR|SOUTHCLINIC|20260925100000+0200||MDM^T02^MDM_T02|MSG00023|P|2.5.1",
            "EVN||20260925095900+0200",
            "PID|1||PAT-0023^^^NORTHHOSP^MR||Doe^Sam^^^^^L||19751111|U",
            "PV1|1|O",
            "TXA|1|CN|TX|20260925095000+0200||||||||DOC-0023|||||AU",
        ]);
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DocumentReference", "masterIdentifier.value"),
            vec![text("DOC-0023")],
            "{:?}",
            writes_of(&run, "DocumentReference")
        );
        assert_eq!(ei_refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: no specification governs this: our own design; two variants equally specific for a
    // value that write it differently are counted with both named, and neither writes.
    #[test]
    fn two_equally_specific_variants_that_differ_are_a_counted_ambiguity() {
        let rival = datatype_map(
            "datatype-ei-rival-to-identifier",
            "http://example.org/fhir/ConceptMap/datatype-ei-rival-to-identifier",
            &[("EI.1", "value", "string"), ("EI.2", "type.text", "string")],
        );
        let corpus = supplemented(&[rival]);
        let parsed = order("ORC|RE|PLC-1^ORDERDESK|FIL-1");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("FIL-1")]
        );
        let ambiguous: Vec<(&str, &[String])> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::DatatypeAmbiguous {
                    row, candidates, ..
                } => Some((row.source.as_str(), candidates.as_slice())),
                _ => None,
            })
            .collect();
        assert_eq!(
            ambiguous,
            vec![(
                "ORC-2",
                &[
                    String::from("datatype-ei-organization-to-identifier"),
                    String::from("datatype-ei-rival-to-identifier"),
                ][..]
            )]
        );
    }

    // NOTE: `StructureDefinition-TypeInfo.json` `mappedVia` is the "Url of the mapping artifact
    // for the item", so a row naming a data type map runs that map alone.
    #[test]
    fn a_row_naming_its_data_type_map_runs_that_map() {
        let segment: Value = serde_json::from_value(serde_json::json!({
            "resourceType": "ConceptMap",
            "id": "segment-orc-to-diagnosticreport",
            "url": "http://example.org/fhir/ConceptMap/segment-orc-to-diagnosticreport",
            "status": "active",
            "group": [{ "element": [{
                "code": "ORC-2",
                "extension": [{
                    "url": crate::map::corpus::TYPE_INFO,
                    "extension": [{ "url": "type", "valueCode": "EI" }]
                }],
                "target": [{
                    "code": "identifier[1]",
                    "equivalence": "equivalent",
                    "extension": [{
                        "url": crate::map::corpus::TYPE_INFO,
                        "extension": [
                            { "url": "type", "valueCode": "Identifier" },
                            {
                                "url": "mappedVia",
                                "valueUrl": "ConceptMap/datatype-ei-extension-to-identifier"
                            }
                        ]
                    }]
                }]
            }]}]
        }))
        .expect("a JSON value");
        let corpus = supplemented(&[segment]);
        let parsed = order("ORC|RE|PLC-1^ORDERDESK");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("PLC-1")]
        );
        assert!(
            values_at(&run, "Organization", "identifier.value")
                .iter()
                .all(|value| value != &text("ORDERDESK")),
            "{:?}",
            writes_of(&run, "Organization")
        );
        assert!(
            run.outcomes.iter().any(|outcome| matches!(
                outcome,
                Outcome::UnmappedComponent { map, component: 2, .. }
                    if map == "datatype-ei-extension-to-identifier"
            )),
            "{:?}",
            run.outcomes
        );
    }

    #[test]
    fn two_alternatives_of_one_extension_value_share_the_choice_instance() {
        let string = resolve_path("Observation", &["extension", "valueString"]).expect("a path");
        let concept = resolve_path(
            "Observation",
            &["extension", "valueCodeableConcept", "coding", "code"],
        )
        .expect("a path");
        let choice = (String::from(".extension.value"), vec![0]);
        assert_eq!(
            alternatives(&string, &[0]),
            vec![(choice.clone(), String::from("valueString"))]
        );
        assert_eq!(
            alternatives(&concept, &[0, 0]),
            vec![(choice, String::from("valueCodeableConcept"))]
        );
        assert_eq!(
            alternatives(&concept, &[1, 0]),
            vec![(
                (String::from(".extension.value"), vec![1]),
                String::from("valueCodeableConcept")
            )]
        );
    }

    /// A synthetic ORU^R01 whose one order carries `orc`, one `OBX` and `spm`.
    fn specimen(orc: &str, obx: &str, spm: &str) -> Parsed {
        message(&[
            "MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00031|P|2.5.1",
            "PID|1||PAT-0031^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
            orc,
            "OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
            obx,
            spm,
        ])
    }

    /// The glucose result `OBX` every specimen case carries.
    const GLUCOSE: &str = "OBX|1|NM|2345-7^Glucose^LN||5.5|mmol/L|||||F";

    /// The maps a run chose as halves, with the row source and target.
    fn halves(run: &Run<'_>) -> Vec<(String, String, String)> {
        run.outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::DatatypeHalf { row, map, .. } => {
                    Some((row.source.clone(), row.target.clone(), map.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// The conflicts and ambiguities a run counted.
    fn refusals<'r>(run: &'r Run<'_>) -> Vec<&'r Outcome> {
        run.outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    Outcome::DatatypeConflict { .. } | Outcome::DatatypeAmbiguous { .. }
                )
            })
            .collect()
    }

    // NOTE: `ConceptMap-datatype-eip-*-to-identifier`: the placer map writes `EIP.1` into `value`
    // and the filler map `EIP.2`, so the two fill one `Identifier` each and are no alternatives.
    #[test]
    fn the_eip_maps_are_halves_and_the_ei_variants_are_not() {
        let corpus = corpus();
        let map = |id: &str| corpus.get(id).expect("a vendored map");
        let placer = map("datatype-eip-placerassignedidentifier-to-identifier");
        let filler = map("datatype-eip-fillerassignedidentifier-to-identifier");
        assert!(placer.half_of(filler));
        assert!(!placer.alternative_to(filler));
        assert!(
            !map("datatype-ei-organization-to-identifier")
                .half_of(map("datatype-ei-system-to-identifier"))
        );
        assert!(
            !map("datatype-hd-endpoint-to-messageheader-source")
                .half_of(map("datatype-hd-name-to-messageheader-source"))
        );
    }

    // NOTE: `segment-spm-to-specimen` maps SPM-2 into `identifier[1]` and `identifier[2]`, and the
    // EIP components are the placer's (EIP.1) then the filler's (EIP.2), in that order.
    #[test]
    fn a_specimen_id_gives_the_placer_and_the_filler_identifier() {
        let corpus = corpus();
        let parsed = specimen("ORC|RE|PLC-1|FIL-1", GLUCOSE, "SPM|1|SPC-P^SPC-F");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "Specimen", "identifier.value"),
            vec![text("SPC-P"), text("SPC-F")],
            "{:?}",
            writes_of(&run, "Specimen")
        );
        assert_eq!(
            values_at(&run, "Specimen", "identifier.type.coding.code"),
            vec![text("PGN"), text("FGN")]
        );
        assert_eq!(
            halves(&run),
            vec![
                (
                    String::from("SPM-2"),
                    String::from("identifier[1]"),
                    String::from("datatype-eip-placerassignedidentifier-to-identifier")
                ),
                (
                    String::from("SPM-2"),
                    String::from("identifier[2]"),
                    String::from("datatype-eip-fillerassignedidentifier-to-identifier")
                ),
            ]
        );
        assert_eq!(refusals(&run), Vec::<&Outcome>::new());
    }

    #[test]
    fn a_filler_specimen_id_alone_gives_the_filler_identifier() {
        let corpus = corpus();
        let parsed = specimen("ORC|RE|PLC-1|FIL-1", GLUCOSE, "SPM|1|^SPC-F");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "Specimen", "identifier.value"),
            vec![text("SPC-F")],
            "{:?}",
            writes_of(&run, "Specimen")
        );
        assert_eq!(
            values_at(&run, "Specimen", "identifier.type.coding.code"),
            vec![text("FGN")]
        );
        assert!(
            run.outcomes.iter().all(
                |outcome| !matches!(outcome, Outcome::UnmappedComponent { map, .. }
                    if map.starts_with("datatype-eip-"))
            ),
            "{:?}",
            run.outcomes
        );
        assert_eq!(refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: `segment-orc-to-diagnosticreport` maps ORC-4 into `identifier[3]` and `identifier[4]`,
    // so a placer group number alone gives the `PGN` identifier and no `FGN` one.
    #[test]
    fn a_placer_group_number_gives_the_placer_identifier() {
        let corpus = corpus();
        let parsed = order("ORC|RE|PLC-1|FIL-1|GRP-P");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert_eq!(
            values_at(&run, "DiagnosticReport", "identifier.value"),
            vec![text("PLC-1"), text("FIL-1"), text("GRP-P")],
            "{:?}",
            writes_of(&run, "DiagnosticReport")
        );
        let codes = values_at(&run, "DiagnosticReport", "identifier.type.coding.code");
        assert!(codes.contains(&text("PGN")), "{codes:?}");
        assert!(!codes.contains(&text("FGN")), "{codes:?}");
        assert_eq!(refusals(&run), Vec::<&Outcome>::new());
    }

    // NOTE: no specification governs this: our own design; an ORC-4 valuing EI components 3
    // and 4 is no EIP either half reads, so the row stays a counted conflict.
    #[test]
    fn an_entity_identifier_where_the_row_names_an_eip_stays_a_conflict() {
        let corpus = corpus();
        let parsed = order("ORC|RE|PLC-1|FIL-1|GRP-1^LAB^1.2.3^ISO");
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert!(
            !values_at(&run, "DiagnosticReport", "identifier.value").contains(&text("GRP-1")),
            "{:?}",
            writes_of(&run, "DiagnosticReport")
        );
        assert_eq!(halves(&run), Vec::new());
        assert!(
            refusals(&run).iter().all(|outcome| matches!(
                outcome,
                Outcome::DatatypeConflict { row, .. } if row.source == "ORC-4"
            )) && !refusals(&run).is_empty(),
            "{:?}",
            run.outcomes
        );
    }

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
}
