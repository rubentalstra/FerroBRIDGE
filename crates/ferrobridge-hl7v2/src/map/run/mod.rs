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

mod alternative;
mod datatype;
mod datum;
mod endpoint;
mod finish;
mod grouping;
mod resource;
mod segment;
mod sibling;
mod value;
mod walk;
mod write;

use std::collections::BTreeSet;

use fhir_types::codec::Value;

use crate::map::condition::{self, Operand, Probe};
use crate::map::corpus::{Corpus, TableGroup};
use crate::map::notation::Label;
use crate::map::run::datum::Datum;
use crate::map::run::walk::collect;
use crate::map::{MapError, Outcome, RowRef};
use crate::parse::{Item, Location, Parsed, Segment};

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
    Translation {
        request: usize,
        part: Part,
    },
    /// A reference to the `label` sibling of the family `anchor` heads, settled
    /// when the run finishes ([`Run::siblings`]), since only then is it known
    /// which siblings a value reached.
    Sibling {
        anchor: usize,
        label: u32,
        map: String,
    },
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
    /// The `label` sibling of the created resource `anchor`, which a data
    /// type map's `[k].` row names ([`Run::family`]).
    Sibling { anchor: usize, label: u32 },
}

/// One resource the run builds.
#[derive(Debug, Clone)]
struct Resource {
    type_name: String,
    identity: Identity,
    full_url: String,
    writes: Vec<Write>,
    /// Whether the resource stays out of the Bundle: a sibling no value
    /// reached, or an empty anchor its references moved away from.
    omitted: bool,
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fhir_types::codec::Value;

    use super::{Pending, Run};
    use crate::decode::Charset;
    use crate::map::Outcome;
    use crate::map::corpus::Corpus;
    use crate::parse::{self, Parsed};

    /// A synthetic result whose MSH carries `header` as MSH-3 to MSH-6.
    pub(super) fn parsed(header: &str) -> Parsed {
        parsed_with(header, "")
    }

    /// A synthetic result as [`parsed`], with `tail` after MSH-12.
    pub(super) fn parsed_with(header: &str, tail: &str) -> Parsed {
        let text = format!(
            "MSH|^~\\&|{header}|20260925143000+0200||ORU^R01^ORU_R01|MSG00011|P|2.5.1{tail}\r\
             PID|1||PAT-0011^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M\r\
             OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN\r"
        );
        let lexed = parse::lex::lex(&text, Charset::Ascii).expect("it lexes");
        let structure = parse::structure::structure_for(&lexed.message).expect("a structure");
        parse::grouping::group(lexed, structure)
    }

    pub(super) fn corpus() -> Corpus {
        let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/package");
        Corpus::load(&package).expect("the vendored package loads")
    }

    /// The `MessageHeader` writes of a run, as dotted paths and ready values.
    pub(super) fn header_writes(run: &Run<'_>) -> Vec<(String, Value)> {
        writes_of(run, "MessageHeader")
    }

    /// The writes of a run into resources of `type_name`, as dotted paths and
    /// ready values.
    pub(super) fn writes_of(run: &Run<'_>, type_name: &str) -> Vec<(String, Value)> {
        run.resources
            .iter()
            .filter(|resource| resource.type_name == type_name)
            .flat_map(|resource| &resource.writes)
            .filter_map(|write| {
                let path: Vec<&str> = write.slots.iter().map(|slot| slot.name.as_str()).collect();
                match &write.value {
                    Pending::Ready(value) => Some((path.join("."), value.clone())),
                    Pending::Translation { .. } | Pending::Sibling { .. } => None,
                }
            })
            .collect()
    }

    /// A synthetic 2.3 ORM^O01 with a laboratory order carrying a note, a
    /// diagnosis and a result, and a pharmacy order.
    pub(super) fn legacy_order() -> Parsed {
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

    /// A data type map with one row per `(component, child, type)` triple, at
    /// the canonical url `url`.
    pub(super) fn datatype_map(id: &str, url: &str, rows: &[(&str, &str, &str)]) -> Value {
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
    pub(super) fn supplemented(maps: &[Value]) -> Corpus {
        let directory = tempfile::tempdir().expect("a temporary directory");
        for (index, map) in maps.iter().enumerate() {
            let path = directory.path().join(format!("ConceptMap-{index}.json"));
            std::fs::write(path, serde_json::to_vec(map).expect("it serializes")).expect("written");
        }
        corpus()
            .supplement(directory.path())
            .expect("the supplement loads")
    }

    /// A synthetic message of `segments`, each ended by a carriage return.
    pub(super) fn message(segments: &[&str]) -> Parsed {
        let text = format!("{}\r", segments.join("\r"));
        let lexed = parse::lex::lex(&text, Charset::Ascii).expect("it lexes");
        let structure = parse::structure::structure_for(&lexed.message).expect("a structure");
        parse::grouping::group(lexed, structure)
    }

    /// The values written at `path` in resources of `type_name`.
    pub(super) fn values_at(run: &Run<'_>, type_name: &str, path: &str) -> Vec<Value> {
        writes_of(run, type_name)
            .into_iter()
            .filter(|(written, _)| written == path)
            .map(|(_, value)| value)
            .collect()
    }

    pub(super) fn text(value: &str) -> Value {
        Value::String(String::from(value))
    }

    /// The conflicts, ambiguities and missing `ST` maps a run counted, the
    /// refusals an EI value can meet.
    pub(super) fn ei_refusals<'r>(run: &'r Run<'_>) -> Vec<&'r Outcome> {
        run.outcomes
            .iter()
            .filter(|outcome| match outcome {
                Outcome::DatatypeConflict { .. } | Outcome::DatatypeAmbiguous { .. } => true,
                Outcome::NoDatatypeMap { row, .. } => row.map == "datatype-st-to-identifier",
                _ => false,
            })
            .collect()
    }

    /// A synthetic ORU^R01 whose one order carries `orc`, one `OBX` and `spm`.
    pub(super) fn specimen(orc: &str, obx: &str, spm: &str) -> Parsed {
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
    pub(super) const GLUCOSE: &str = "OBX|1|NM|2345-7^Glucose^LN||5.5|mmol/L|||||F";

    /// The vendored package with the crate's shipped supplements over it.
    pub(super) fn shipped() -> Corpus {
        corpus()
            .with_shipped_supplements()
            .expect("the shipped supplements load")
    }
}
