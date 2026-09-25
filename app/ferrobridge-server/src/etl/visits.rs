// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The derived visits: the rows of the configured visit query, grouped by
//! EHR and source, each spanning the earliest start to the latest end.
//!
//! A start or an end is an ISO 8601 date or date and time as openEHR writes
//! a `DV_DATE_TIME` value. The CDM `datetime` carries no zone, so a value
//! with an offset keeps its local wall-clock time and drops the offset, and
//! the earliest and latest are compared on that wall-clock time. A value
//! without a full calendar date refuses its row: a required date is never
//! filled in. No specification governs the grouping or the reading: our own
//! design (the CDM leaves visits to the ETL,
//! <https://ohdsi.github.io/CommonDataModel/cdm54.html#visit_occurrence>).

use crate::etl::aql::CheckedQuery;
use omop_cdm::graph::{EhrId, MappingName, Refusal, Visit, VisitKey, VisitSource};
use omop_cdm::value::{CdmDate, CdmDatetime};
use std::collections::BTreeMap;

/// The mapping name the report files visit refusals under.
pub const VISIT_MAPPING: &str = "visits";

/// One end of a visit as the source wrote it, read into CDM forms.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Moment {
    /// The wall-clock form, `YYYY-MM-DD` or `YYYY-MM-DDThh:mm:ss[.f]`, which
    /// orders the moments.
    local: String,
    /// The date.
    date: CdmDate,
    /// The date and time, when the value carries a time.
    datetime: Option<CdmDatetime>,
}

/// Returns `text` without a trailing `Z` or `±hh:mm` offset on its time.
///
/// The cut is lexical so the fraction of a second keeps the digits the source
/// wrote: `openehr_base::v1_3::foundation_types::time::iso8601_date_time`
/// yields the fraction as an `f64`, which cannot carry them.
fn without_offset(text: &str) -> &str {
    let Some((date, time)) = text.split_once('T') else {
        return text;
    };
    let cut = time
        .find(['Z', '+', '-'])
        .map_or(text.len(), |at| date.len() + 1 + at);
    text.get(..cut).unwrap_or(text)
}

/// Reads `text` as a moment, or `None` when it carries no full date.
fn moment(text: &str) -> Option<Moment> {
    let local = without_offset(text);
    let date = CdmDate::new(local.get(..10)?).ok()?;
    // NOTE: no specification governs this: our own design; a value that is no
    // CDM datetime after its date is refused whole rather than cut to its date.
    let datetime = if local.len() > 10 {
        Some(CdmDatetime::new(local).ok()?)
    } else {
        None
    };
    Some(Moment {
        local: local.to_owned(),
        date,
        datetime,
    })
}

/// The visits one query answered, and the rows it refused.
#[derive(Debug, Default)]
pub struct Visits {
    /// The visits, by key.
    pub visits: Vec<Visit>,
    /// The rows refused, and why.
    pub refusals: Vec<Refusal>,
}

/// Returns a refusal of a visit row.
fn refused(reason: String) -> Refusal {
    let refusal = Refusal::new(reason);
    match MappingName::new(VISIT_MAPPING) {
        Ok(mapping) => refusal.with_mapping(mapping),
        Err(_) => refusal,
    }
}

/// Returns the text of cell `projection` of `row`.
fn text<'r>(
    query: &CheckedQuery,
    row: &'r [serde_json::Value],
    projection: &'static str,
) -> Result<&'r str, Refusal> {
    query
        .column(projection)
        .and_then(|position| row.get(position))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| refused(format!("the visit row carries no text `{projection}`")))
}

/// Groups `rows` of `query` into visits.
#[must_use]
pub fn group(query: &CheckedQuery, rows: &[Vec<serde_json::Value>]) -> Visits {
    let mut spans: BTreeMap<VisitKey, (Moment, Moment)> = BTreeMap::new();
    let mut refusals = Vec::new();
    for row in rows {
        let read = || -> Result<(VisitKey, Moment, Moment), Refusal> {
            let ehr = EhrId::new(text(query, row, "ehr_id")?)
                .map_err(|error| refused(format!("the visit row's ehr_id: {error}")))?;
            let source = VisitSource::new(text(query, row, "visit_source")?)
                .map_err(|error| refused(format!("the visit row's visit_source: {error}")))?;
            let start = moment(text(query, row, "visit_start")?).ok_or_else(|| {
                refused(String::from("visit_start carries no date the CDM can hold"))
                    .with_element("visit_start")
            })?;
            let end = moment(text(query, row, "visit_end")?).ok_or_else(|| {
                refused(String::from("visit_end carries no date the CDM can hold"))
                    .with_element("visit_end")
            })?;
            Ok((VisitKey::new(ehr, source), start, end))
        };
        match read() {
            Ok((key, start, end)) => {
                let span = spans
                    .entry(key)
                    .or_insert_with(|| (start.clone(), end.clone()));
                if start < span.0 {
                    span.0 = start;
                }
                if end > span.1 {
                    span.1 = end;
                }
            }
            Err(refusal) => refusals.push(refusal),
        }
    }
    let mut visits = Vec::with_capacity(spans.len());
    for (key, (start, end)) in spans {
        match Visit::new(key, (start.date, start.datetime), (end.date, end.datetime)) {
            Ok(visit) => visits.push(visit),
            Err(error) => refusals.push(refused(error.to_string())),
        }
    }
    Visits { visits, refusals }
}

#[cfg(test)]
mod tests {
    use super::{group, moment, without_offset};
    use crate::etl::aql::CheckedQuery;
    use serde_json::json;

    /// A visit query with the four projections.
    const VISITS: &str = "SELECT e/ehr_id/value AS ehr_id, c/context/other_context/items[at0001]/value/value AS visit_source, \
        c/context/start_time/value AS visit_start, c/context/end_time/value AS visit_end \
        FROM EHR e CONTAINS COMPOSITION c ORDER BY c/context/start_time/value";

    #[test]
    fn an_offset_is_dropped_and_the_wall_clock_time_kept() {
        assert_eq!(
            "2026-06-14T10:00:00",
            without_offset("2026-06-14T10:00:00+02:00")
        );
        assert_eq!(
            "2026-06-14T10:00:00.5",
            without_offset("2026-06-14T10:00:00.5Z")
        );
        assert_eq!("2026-06-14", without_offset("2026-06-14"));
        assert!(moment("2026-06").is_none(), "a partial date is no CDM date");
    }

    #[test]
    fn rows_group_by_ehr_and_source_from_the_first_start_to_the_last_end() {
        let query = CheckedQuery::visits(VISITS).expect("the query checks");
        let rows = vec![
            vec![
                json!("ehr-1"),
                json!("enc-1"),
                json!("2026-06-14T10:00:00+02:00"),
                json!("2026-06-14T11:00:00+02:00"),
            ],
            vec![
                json!("ehr-1"),
                json!("enc-1"),
                json!("2026-06-13T09:00:00+02:00"),
                json!("2026-06-15T08:00:00+02:00"),
            ],
            vec![
                json!("ehr-1"),
                json!("enc-2"),
                json!("2026-06-20"),
                json!("2026-06-21"),
            ],
            vec![
                json!("ehr-1"),
                json!("enc-3"),
                json!("2026-06-20"),
                serde_json::Value::Null,
            ],
        ];
        let visits = group(&query, &rows);
        assert_eq!(2, visits.visits.len());
        let first = visits.visits.first().expect("one visit");
        assert_eq!("2026-06-13", first.start().as_str());
        assert_eq!("2026-06-15", first.end().as_str());
        assert_eq!(
            Some("2026-06-15T08:00:00"),
            first
                .end_datetime()
                .map(omop_cdm::value::CdmDatetime::as_str)
        );
        assert_eq!(
            1,
            visits.refusals.len(),
            "the visit without an end is refused"
        );
    }
}
