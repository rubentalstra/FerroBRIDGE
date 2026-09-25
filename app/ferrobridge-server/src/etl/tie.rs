// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The tie of a composition to a derived visit.
//!
//! A composition belongs to the visit of its own EHR whose window contains
//! its moment: the composition's `context/start_time`, or, for a composition
//! without a context, the first date the mapping resolved on its first row.
//! When several windows contain the moment, the visit whose source equals the
//! name of the composition's `context/health_care_facility` wins; when that
//! does not decide, the composition carries no visit and the run counts it as
//! ambiguous, as it counts a composition inside no window. Moments are read
//! as wall-clock times to the second, the offset dropped, as the visits are
//! (`etl::visits`); where one side carries no time, the dates are compared.
//! The CDM asks every clinical event to carry its visit where one exists
//! (<https://ohdsi.github.io/CommonDataModel/cdm54.html#VISIT_OCCURRENCE>)
//! and leaves the tie to the ETL. No specification governs the rule, its
//! reading of time or its tie-breaker: our own design.

use omop_cdm::graph::{Cell, RecordGraph, Refusal, Value, Visit, VisitKey};
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::foundation_types::time::iso8601_date_time::Iso8601DateTime;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::composition::event_context::EventContext;
use std::collections::BTreeMap;

/// A wall-clock moment: a calendar date and, when known, a time of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wall {
    /// Year, month and day.
    date: (u32, u32, u32),
    /// Hour, minute and second, when the value carries a time.
    time: Option<(u32, u32, u32)>,
}

impl Wall {
    /// Reads an ISO 8601 date or date and time through `openehr-base`, or
    /// `None` when the value carries no full calendar date.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        let parsed = Iso8601DateTime {
            value: text.to_owned(),
        };
        let date = (parsed.year()?, parsed.month()?, parsed.day()?);
        // NOTE: no specification governs this: our own design; a missing minute
        // or second reads as 00 and a fractional second is not compared.
        let time = parsed.hour().map(|hour| {
            (
                hour,
                parsed.minute().unwrap_or(0),
                parsed.second().unwrap_or(0),
            )
        });
        Some(Self { date, time })
    }

    /// Returns whether `self` falls on or after `start`.
    fn on_or_after(self, start: Self) -> bool {
        match (self.time, start.time) {
            (Some(time), Some(bound)) => (self.date, time) >= (start.date, bound),
            _ => self.date >= start.date,
        }
    }

    /// Returns whether `self` falls on or before `end`.
    fn on_or_before(self, end: Self) -> bool {
        match (self.time, end.time) {
            (Some(time), Some(bound)) => (self.date, time) <= (end.date, bound),
            _ => self.date <= end.date,
        }
    }
}

/// What the tie of one composition reads from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// The moment, when the composition carries a context start time.
    pub moment: Option<Wall>,
    /// The name of the context's health care facility, when it carries one.
    pub facility: Option<String>,
}

/// Reads the context start time and the facility name of `composition`.
///
/// A composition without a `context`, legitimately a persistent one (RM
/// Composition, `COMPOSITION.context`), has no anchor moment.
///
/// # Errors
///
/// Returns a [`Refusal`] when the `context` does not read as an RM
/// `EVENT_CONTEXT`.
pub fn anchor(composition: &CanonicalComposition) -> Result<Anchor, Refusal> {
    let Some(context) = composition.value().get("context") else {
        return Ok(Anchor {
            moment: None,
            facility: None,
        });
    };
    let context: EventContext = serde_json::from_value(context.clone()).map_err(|error| {
        Refusal::new(format!(
            "the composition's context is no EVENT_CONTEXT: {error}"
        ))
        .with_element("/context")
    })?;
    let facility = context
        .health_care_facility
        .as_ref()
        .and_then(|party| match party {
            PartyIdentified::PartyIdentified(data) => data.name.clone(),
            PartyIdentified::PartyRelated(related) => related.name.clone(),
        });
    Ok(Anchor {
        moment: Wall::read(&context.start_time.value),
        facility,
    })
}

/// Returns the first date the mapping resolved on the first row that carries
/// one, its time too when the row's matching `_datetime` column holds one.
#[must_use]
pub fn resolved_moment(graph: &RecordGraph) -> Option<Wall> {
    graph.rows().iter().find_map(|row| {
        row.table().columns.iter().find_map(|column| {
            let Some(Cell::Value(Value::Date(date))) = row.cell(column.name) else {
                return None;
            };
            let datetime = row.cell(&format!("{}time", column.name));
            match datetime {
                Some(Cell::Value(Value::Datetime(datetime))) => Wall::read(datetime.as_str()),
                _ => Wall::read(date.as_str()),
            }
        })
    })
}

/// The outcome of one tie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tie {
    /// The composition belongs to this visit.
    Visit(VisitKey),
    /// No visit of the EHR contains the moment, or there is no moment.
    Outside,
    /// Several visits contain the moment and the facility does not decide.
    Ambiguous,
}

impl Tie {
    /// Returns the visit, when the tie found one.
    #[must_use]
    pub fn visit(&self) -> Option<&VisitKey> {
        match self {
            Self::Visit(key) => Some(key),
            Self::Outside | Self::Ambiguous => None,
        }
    }
}

/// One visit window.
#[derive(Debug, Clone)]
struct Window {
    key: VisitKey,
    start: Wall,
    end: Wall,
}

/// The derived visits of a run, by EHR, keyed by the `ehr_id` as written.
#[derive(Debug, Clone, Default)]
pub struct Windows {
    by_ehr: BTreeMap<String, Vec<Window>>,
}

/// Returns the wall-clock form of one bound of a visit.
fn bound(date: &str, datetime: Option<&str>) -> Option<Wall> {
    Wall::read(datetime.unwrap_or(date))
}

impl Windows {
    /// Indexes `visits` by EHR.
    #[must_use]
    pub fn new(visits: &[Visit]) -> Self {
        let mut by_ehr: BTreeMap<String, Vec<Window>> = BTreeMap::new();
        for visit in visits {
            let start = bound(
                visit.start().as_str(),
                visit
                    .start_datetime()
                    .map(omop_cdm::value::CdmDatetime::as_str),
            );
            let end = bound(
                visit.end().as_str(),
                visit
                    .end_datetime()
                    .map(omop_cdm::value::CdmDatetime::as_str),
            );
            // NOTE: a written visit's CDM date always reads back, so a window
            // that does not is legitimately absent rather than defective.
            if let (Some(start), Some(end)) = (start, end) {
                by_ehr
                    .entry(visit.key().ehr_id().value().to_owned())
                    .or_default()
                    .push(Window {
                        key: visit.key().clone(),
                        start,
                        end,
                    });
            }
        }
        Self { by_ehr }
    }

    /// Ties a composition of `ehr` at `moment` to a visit, with `facility`
    /// deciding between several.
    #[must_use]
    pub fn tie(&self, ehr: &HierObjectId, moment: Wall, facility: Option<&str>) -> Tie {
        let containing: Vec<&Window> = self
            .by_ehr
            .get(ehr.value())
            .map(|windows| {
                windows
                    .iter()
                    .filter(|window| {
                        moment.on_or_after(window.start) && moment.on_or_before(window.end)
                    })
                    .collect()
            })
            .unwrap_or_default();
        match containing.as_slice() {
            [] => Tie::Outside,
            [only] => Tie::Visit(only.key.clone()),
            several => {
                let named: Vec<&&Window> = several
                    .iter()
                    .filter(|window| Some(window.key.source().as_str()) == facility)
                    .collect();
                match named.as_slice() {
                    [only] => Tie::Visit(only.key.clone()),
                    _ => Tie::Ambiguous,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Tie, Wall, Windows, anchor, resolved_moment};
    use omop_cdm::graph::{
        ArchetypeRootPath, Discriminator, MappingName, OccurrencePath, RecordGraph, RecordKey,
        Reference, Row, RowBuilder, Source, Value, Visit, VisitKey, VisitSource,
    };
    use omop_cdm::value::{CdmDate, CdmDatetime};
    use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
    use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
    use openehr_mapping_core::composition::CanonicalComposition;
    use openehr_mapping_core::template::Generation;
    use serde_json::json;

    /// Returns a visit of `ehr-1` under `source` between two datetimes.
    fn visit(source: &str, start: &str, end: &str) -> Visit {
        let date = |text: &str| CdmDate::new(text.get(..10).expect("a date")).expect("a CDM date");
        Visit::new(
            VisitKey::new(
                HierObjectId::new("ehr-1").expect("an id"),
                VisitSource::new(source).expect("a source"),
            ),
            (
                date(start),
                Some(CdmDatetime::new(start).expect("a datetime")),
            ),
            (date(end), Some(CdmDatetime::new(end).expect("a datetime"))),
        )
        .expect("a visit")
    }

    #[test]
    fn an_offset_is_dropped_and_a_partial_date_has_no_moment() {
        let wall = Wall::read("2026-06-14T10:00:00+02:00").expect("a moment");
        assert_eq!(Wall::read("2026-06-14T10:00:00Z"), Some(wall));
        assert_eq!(None, Wall::read("2026-06"));
        assert!(Wall::read("2026-06-14").is_some_and(|wall| wall.time.is_none()));
    }

    #[test]
    fn the_window_bounds_are_inclusive_and_a_date_compares_by_day() {
        let windows = Windows::new(&[visit("a", "2026-06-13T08:00:00", "2026-06-14T12:00:00")]);
        let ehr = HierObjectId::new("ehr-1").expect("an id");
        let at = |text: &str| windows.tie(&ehr, Wall::read(text).expect("a moment"), None);
        assert!(matches!(at("2026-06-13T08:00:00"), Tie::Visit(_)));
        assert!(matches!(at("2026-06-14T12:00:00Z"), Tie::Visit(_)));
        assert_eq!(Tie::Outside, at("2026-06-14T12:00:01"));
        assert!(
            matches!(at("2026-06-13"), Tie::Visit(_)),
            "a date-only moment"
        );
        assert_eq!(
            Tie::Outside,
            windows.tie(
                &HierObjectId::new("ehr-2").expect("an id"),
                Wall::read("2026-06-13T09:00:00").expect("a moment"),
                None
            ),
            "another EHR's visit never ties"
        );
    }

    #[test]
    fn the_facility_decides_between_two_windows_and_nothing_else_does() {
        let windows = Windows::new(&[
            visit("ward-a", "2026-06-13T08:00:00", "2026-06-16T12:00:00"),
            visit("ward-b", "2026-06-14T08:00:00", "2026-06-15T12:00:00"),
        ]);
        let ehr = HierObjectId::new("ehr-1").expect("an id");
        let moment = Wall::read("2026-06-14T10:00:00").expect("a moment");
        assert_eq!(
            Some("ward-b"),
            windows
                .tie(&ehr, moment, Some("ward-b"))
                .visit()
                .map(|key| key.source().as_str())
        );
        assert_eq!(Tie::Ambiguous, windows.tie(&ehr, moment, None));
        assert_eq!(Tie::Ambiguous, windows.tie(&ehr, moment, Some("ward-c")));
    }

    /// Wraps canonical JSON as a composition of a synthetic template.
    fn canonical(value: serde_json::Value) -> CanonicalComposition {
        CanonicalComposition::new(value, "ferrobridge.synthetic.v1", Generation::Adl14)
    }

    #[test]
    fn the_anchor_is_the_context_start_and_the_facility_name() {
        let anchored = anchor(&canonical(json!({
            "context": {
                "_type": "EVENT_CONTEXT",
                "start_time": {"_type": "DV_DATE_TIME", "value": "2026-06-14T10:00:00+02:00"},
                "setting": {
                    "_type": "DV_CODED_TEXT",
                    "value": "other care",
                    "defining_code": {
                        "_type": "CODE_PHRASE",
                        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"},
                        "code_string": "238"
                    }
                },
                "health_care_facility": {"_type": "PARTY_IDENTIFIED", "name": "ward-a"}
            }
        })))
        .expect("the context reads");
        assert_eq!(Wall::read("2026-06-14T10:00:00"), anchored.moment);
        assert_eq!(Some("ward-a"), anchored.facility.as_deref());
        let persistent = anchor(&canonical(json!({}))).expect("no context is no anchor");
        assert_eq!(None, persistent.moment);
        assert!(
            anchor(&canonical(json!({"context": {"_type": "EVENT_CONTEXT"}}))).is_err(),
            "a context without its start time is refused"
        );
    }

    #[test]
    fn the_resolved_moment_is_the_first_rows_first_date_with_its_time() {
        let ehr = HierObjectId::new("ehr-1").expect("an id");
        let source = Source::new(
            ehr.clone(),
            ObjectVersionId::new("vo-1::synthetic::1").expect("an id"),
        );
        let key = RecordKey::new(
            &source,
            ArchetypeRootPath::new("/content[openEHR-EHR-OBSERVATION.synthetic.v1]")
                .expect("a root"),
            OccurrencePath::new("/").expect("a path"),
            Discriminator::new(MappingName::new("Synthetic_v1").expect("a name"), 0, 0),
        );
        let row = Row::builder("measurement", key)
            .and_then(|builder| builder.reference("person_id", Reference::Person(ehr.clone())))
            .and_then(|builder| builder.value("measurement_concept_id", Value::Integer(0)))
            .and_then(|builder| builder.value("measurement_type_concept_id", Value::Integer(0)))
            .and_then(|builder| {
                builder.value(
                    "measurement_date",
                    Value::Date(CdmDate::new("2026-06-14").expect("a date")),
                )
            })
            .and_then(|builder| {
                builder.value(
                    "measurement_datetime",
                    Value::Datetime(CdmDatetime::new("2026-06-14T10:30:00").expect("a datetime")),
                )
            })
            .and_then(RowBuilder::build)
            .expect("the row builds");
        let mut graph = RecordGraph::new(source);
        assert_eq!(None, resolved_moment(&graph), "no row, no moment");
        graph.push_row(row).expect("the row fits");
        assert_eq!(Wall::read("2026-06-14T10:30:00"), resolved_moment(&graph));
    }
}
