// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOP ETL run over the CDR: read, map, commit, then derive.
//!
//! The run reads the configured AQL page by page, maps each composition to
//! its record graph, commits it whole, and rebuilds the derived tables at the
//! end (`docs/architecture.md` §5.1, §5.4).
//!
//! The composition query selects each composition whole beside its
//! `ehr_id` and `version_uid`, and optionally its `versioned_object_uid`;
//! the runner reads the template the composition names from the CDR once
//! per run, validates the composition against it, ties it to its derived
//! visit ([`tie`]), and hands it to a [`Mapper`]. A composition the
//! mapper or the writer refuses is rolled back and reported, and the run goes
//! on; a CDR or database that stops answering ends the run with a typed
//! error. A resumed run skips every composition whose watermark names the
//! version the query answered. No specification governs the run: our own
//! design, over openEHR ITS-REST 1.1.0 `POST /query/aql` with `offset` and
//! `fetch` paging.

pub mod aql;
pub mod job;
pub mod mapper;
pub mod report;
pub mod tie;
pub mod visits;

use crate::etl::aql::{CheckedQuery, SINCE_PARAMETER};
use crate::etl::report::RunReport;
use crate::etl::tie::{Tie, Windows};
use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::ids::ObjectVersionId;
use ferrobridge_openehr::query::{PageSize, QueryPageError};
use futures_util::StreamExt;
use omop_cdm::derived;
use omop_cdm::graph::{
    EhrId, RecordGraph, Refusal, Source, VersionUid, VersionedObjectUid, VisitKey,
};
use omop_cdm::writer::{CdmWriter, RunId, VisitConcepts, WriteError};
use openehr_its::rest::generated::query::AdhocQueryExecute;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_rm::v1_2::common::archetyped::archetyped::Archetyped;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

/// The derived visits a run writes before the compositions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisitSettings {
    /// The checked visit query.
    pub query: CheckedQuery,
    /// The concepts every visit carries.
    pub concepts: VisitConcepts,
}

/// The `[etl]` section, resolved: the checked queries and the concepts the
/// deployment configures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtlSettings {
    /// The checked composition query.
    pub compositions: CheckedQuery,
    /// How many rows one page of either query asks for.
    pub page_size: PageSize,
    /// The `*_type_concept_id` the mapper writes: the provenance of the
    /// records, which the deployment knows.
    pub type_concept_id: i32,
    /// The `period_type_concept_id` of every observation period.
    pub observation_period_type_concept_id: i32,
    /// The visit derivation, when configured.
    pub visits: Option<VisitSettings>,
}

/// What a mapper is handed besides the composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappingContext {
    /// The configured `*_type_concept_id`.
    pub type_concept_id: i32,
}

/// One composition version, validated against its template, for a mapper.
#[derive(Debug)]
pub struct SourceComposition<'a> {
    /// The composition version.
    pub source: Source,
    /// The composition.
    pub composition: &'a CanonicalComposition,
    /// The index of the template it was validated against.
    pub template: &'a WebTemplateIndex,
    /// The derived visit the composition belongs to, when the run ties it
    /// to one ([`tie`]).
    pub visit: Option<&'a VisitKey>,
    /// The configured context.
    pub context: MappingContext,
}

/// Maps one composition to the record graph the writer commits.
///
/// A mapper refuses a whole composition with a [`Refusal`]; a record it
/// refuses inside a composition it records in the graph's report instead.
pub trait Mapper {
    /// Maps `composition`.
    fn map(
        &self,
        composition: &SourceComposition<'_>,
    ) -> impl Future<Output = Result<RecordGraph, Refusal>>;
}

/// How one run reads the CDR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOptions {
    /// Skips every composition whose watermark names the version the query
    /// answers.
    pub resume: bool,
    /// The value bound to the composition query's `$since` parameter.
    pub since: Option<String>,
}

/// A run that could not go on.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunError {
    /// `--since` was given to a query without `$since`, or withheld from a
    /// query that names it.
    #[error("{0}")]
    Since(&'static str),
    /// The CDR refused or failed a page of the composition query.
    #[error("the CDR did not answer the composition query")]
    Compositions(#[source] Box<QueryPageError>),
    /// The CDR refused or failed a page of the visit query.
    #[error("the CDR did not answer the visit query")]
    Visits(#[source] Box<QueryPageError>),
    /// The CDR did not answer a template fetch.
    #[error("the CDR did not serve a template the compositions name")]
    Template(#[source] Box<crate::mappings::Error>),
    /// The CDM database refused the visits, a derivation, or a read, or its
    /// connection closed.
    #[error("the CDM database stopped the run")]
    Database(#[source] WriteError),
}

/// Returns the request for `query`, with `$since` bound when given.
fn request(query: &CheckedQuery, since: Option<&str>) -> AdhocQueryExecute {
    AdhocQueryExecute {
        q: query.text().to_owned(),
        offset: None,
        fetch: None,
        query_parameters: since.map(|since| {
            BTreeMap::from([(
                String::from(SINCE_PARAMETER),
                serde_json::Value::String(since.to_owned()),
            )])
        }),
    }
}

/// Returns the text of cell `projection` of a composition row.
fn cell<'r>(
    query: &CheckedQuery,
    row: &'r [serde_json::Value],
    projection: &'static str,
) -> Result<&'r serde_json::Value, Refusal> {
    query
        .column(projection)
        .and_then(|position| row.get(position))
        .filter(|value| !value.is_null())
        .ok_or_else(|| Refusal::new(format!("the composition row carries no `{projection}`")))
}

/// Returns the text of cell `projection`.
fn text<'r>(
    query: &CheckedQuery,
    row: &'r [serde_json::Value],
    projection: &'static str,
) -> Result<&'r str, Refusal> {
    cell(query, row, projection)?
        .as_str()
        .ok_or_else(|| Refusal::new(format!("the composition row's `{projection}` is no text")))
}

/// Reads the composition version a row names.
///
/// The versioned object is the `object_id` part of the `version_uid` (openEHR RM
/// Common 1.1.0, `OBJECT_VERSION_ID`); a `versioned_object_uid` the query selects
/// must name the same one.
fn source(query: &CheckedQuery, row: &[serde_json::Value]) -> Result<Source, Refusal> {
    let ehr = text(query, row, "ehr_id")?;
    let version = text(query, row, "version_uid")?;
    let parsed = ObjectVersionId::new(version)
        .map_err(|error| Refusal::new(format!("the version_uid `{version}`: {error}")))?;
    // NOTE: openEHR RM Common 1.1.0 §OBJECT_VERSION_ID; the versioned object
    // is the `object_id` part of the version's identifier.
    let versioned = parsed.versioned_object_uid();
    if query.column("versioned_object_uid").is_some() {
        let selected = text(query, row, "versioned_object_uid")?;
        if versioned.as_str() != selected {
            return Err(Refusal::new(format!(
                "the version_uid `{version}` is no version of `{selected}`"
            )));
        }
    }
    let refused = |error: omop_cdm::graph::EmptyIdentifier| Refusal::new(error.to_string());
    Ok(Source::new(
        EhrId::new(ehr).map_err(refused)?,
        VersionedObjectUid::new(versioned.as_str()).map_err(refused)?,
        VersionUid::new(version).map_err(refused)?,
    ))
}

/// The templates one run has read from the CDR, by identifier.
type Templates = BTreeMap<String, Arc<WebTemplateIndex>>;

/// Returns the index of the template `composition` names, reading it from
/// the CDR on first use.
async fn template_of(
    cdr: &Client,
    templates: &mut Templates,
    composition: &serde_json::Value,
) -> Result<Result<Arc<WebTemplateIndex>, Refusal>, RunError> {
    let Some(details) = composition.get("archetype_details") else {
        return Ok(Err(Refusal::new(
            "the composition names no archetype_details.template_id",
        )));
    };
    let details = match openehr_its::json::from_canonical_value::<Archetyped>(details) {
        Ok(details) => details,
        Err(error) => {
            return Ok(Err(Refusal::new(format!(
                "the composition's archetype_details do not read as ARCHETYPED: {error}"
            ))));
        }
    };
    let Some(template) = details.template_id.as_ref().map(|id| id.value.as_str()) else {
        return Ok(Err(Refusal::new(
            "the composition names no archetype_details.template_id",
        )));
    };
    if let Some(index) = templates.get(template) {
        return Ok(Ok(Arc::clone(index)));
    }
    match crate::mappings::template_index(cdr, template).await {
        Ok(index) => {
            let index = Arc::new(index);
            templates.insert(template.to_owned(), Arc::clone(&index));
            Ok(Ok(index))
        }
        Err(error @ crate::mappings::Error::TemplateFetch { .. }) => {
            Err(RunError::Template(Box::new(error)))
        }
        Err(error) => Ok(Err(Refusal::new(crate::chain(&error)))),
    }
}

/// Reads every row of `query` from the CDR.
async fn rows(
    cdr: &Client,
    query: &CheckedQuery,
    page: PageSize,
) -> Result<Vec<Vec<serde_json::Value>>, Box<QueryPageError>> {
    let stream = cdr.query_aql_rows(request(query, None), page);
    let mut stream = std::pin::pin!(stream);
    let mut rows = Vec::new();
    while let Some(row) = stream.next().await {
        rows.push(row.map_err(Box::new)?);
    }
    Ok(rows)
}

/// Runs the ETL once: the visits, every composition the query answers, then
/// the derived tables.
///
/// # Errors
///
/// Returns [`RunError::Since`] when `--since` and the query disagree,
/// [`RunError::Compositions`] and [`RunError::Visits`] when the CDR refuses a
/// page, [`RunError::Template`] when it does not answer a template fetch,
/// and [`RunError::Database`] when the CDM database refuses the visits, a
/// watermark read or a derivation, or closes its connection. A composition
/// refused on its own is reported and never ends the run.
pub async fn run<M: Mapper>(
    settings: &EtlSettings,
    options: &RunOptions,
    cdr: &Client,
    writer: &mut CdmWriter,
    mapper: &M,
    run_id: &RunId,
) -> Result<RunReport, RunError> {
    let query = &settings.compositions;
    match (
        options.since.is_some(),
        query.names_parameter(SINCE_PARAMETER),
    ) {
        (true, false) => {
            return Err(RunError::Since(
                "--since was given and the composition query names no $since",
            ));
        }
        (false, true) => {
            return Err(RunError::Since(
                "the composition query names $since and no --since was given",
            ));
        }
        _ => {}
    }
    let mut report = RunReport::new(run_id.as_str());
    let mut windows = None;
    if let Some(visits) = settings.visits.as_ref() {
        let rows = rows(cdr, &visits.query, settings.page_size)
            .await
            .map_err(RunError::Visits)?;
        let grouped = visits::group(&visits.query, &rows);
        for refusal in &grouped.refusals {
            report.refusal(None, refusal);
        }
        report.derived.visit_occurrence = writer
            .write_visits(&grouped.visits, visits.concepts)
            .await
            .map_err(RunError::Database)?;
        windows = Some(Windows::new(&grouped.visits));
    }

    let context = MappingContext {
        type_concept_id: settings.type_concept_id,
    };
    let mut templates = Templates::new();
    let stream = cdr.query_aql_rows(request(query, options.since.as_deref()), settings.page_size);
    let mut stream = std::pin::pin!(stream);
    while let Some(row) = stream.next().await {
        let row = row.map_err(|error| RunError::Compositions(Box::new(error)))?;
        report.compositions.read = report.compositions.read.saturating_add(1);
        let source = match source(query, &row) {
            Ok(source) => source,
            Err(refusal) => {
                report.composition_refused(None, &refusal);
                continue;
            }
        };
        if options.resume
            && let Some(watermark) = writer
                .watermark(source.versioned_object_uid())
                .await
                .map_err(RunError::Database)?
            && watermark.version_uid() == source.version_uid()
        {
            report.compositions.skipped = report.compositions.skipped.saturating_add(1);
            continue;
        }
        let read = Read {
            query,
            row: &row,
            source: &source,
            context,
            windows: windows.as_ref(),
        };
        let (graph, tie) = match composition(cdr, &mut templates, &read, mapper).await? {
            Ok(outcome) => outcome,
            Err(refusal) => {
                report.composition_refused(Some(&source), &refusal);
                continue;
            }
        };
        match writer.commit(&graph, run_id).await {
            Ok(written) => {
                report.committed(&source, &written);
                report.tied(tie.as_ref());
            }
            Err(error) if writer.is_closed() => return Err(RunError::Database(error)),
            Err(error) => {
                report.engine_outcomes(Some(&source), graph.report());
                report.composition_refused(Some(&source), &Refusal::new(crate::chain(&error)));
            }
        }
    }

    report.derived.observation_period =
        derived::observation_period(writer, settings.observation_period_type_concept_id)
            .await
            .map_err(RunError::Database)?;
    report.derived.condition_era = derived::condition_era(writer)
        .await
        .map_err(RunError::Database)?;
    report.derived.drug_era = derived::drug_era(writer)
        .await
        .map_err(RunError::Database)?;
    Ok(report)
}

/// One composition row and what the run knows when it maps it.
struct Read<'r> {
    /// The checked composition query.
    query: &'r CheckedQuery,
    /// The row.
    row: &'r [serde_json::Value],
    /// The composition version the row names.
    source: &'r Source,
    /// The configured context.
    context: MappingContext,
    /// The derived visits, when the run derives them.
    windows: Option<&'r Windows>,
}

/// Reads, validates and maps the composition of one row, tied to its visit
/// when the run derives visits.
async fn composition<M: Mapper>(
    cdr: &Client,
    templates: &mut Templates,
    read: &Read<'_>,
    mapper: &M,
) -> Result<Result<(RecordGraph, Option<Tie>), Refusal>, RunError> {
    let value = match cell(read.query, read.row, "composition") {
        Ok(value) if value.is_object() => value.clone(),
        Ok(_) => {
            return Ok(Err(Refusal::new(
                "the composition row's `composition` is no canonical JSON object",
            )));
        }
        Err(refusal) => return Ok(Err(refusal)),
    };
    let index = match template_of(cdr, templates, &value).await? {
        Ok(index) => index,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let canonical = match index.accept(value) {
        Ok(canonical) => canonical,
        Err(error) => return Ok(Err(Refusal::new(crate::chain(&error)))),
    };
    let Some(windows) = read.windows else {
        return Ok(map(mapper, read, &canonical, &index, None)
            .await
            .map(|graph| (graph, None)));
    };
    Ok(tied(mapper, read, &canonical, &index, windows).await)
}

/// Maps `canonical` tied to the visit its moment falls in.
async fn tied<M: Mapper>(
    mapper: &M,
    read: &Read<'_>,
    canonical: &CanonicalComposition,
    index: &WebTemplateIndex,
    windows: &Windows,
) -> Result<(RecordGraph, Option<Tie>), Refusal> {
    let anchor = tie::anchor(canonical)?;
    let ehr = read.source.ehr_id();
    let facility = anchor.facility.as_deref();
    if let Some(moment) = anchor.moment {
        let tie = windows.tie(ehr, moment, facility);
        let graph = map(mapper, read, canonical, index, tie.visit()).await?;
        return Ok((graph, Some(tie)));
    }
    // NOTE: no specification governs this: our own design; a composition with no
    // context start time is tied by the first date its mapping resolved.
    let graph = map(mapper, read, canonical, index, None).await?;
    let tie = tie::resolved_moment(&graph)
        .map_or(Tie::Outside, |moment| windows.tie(ehr, moment, facility));
    let graph = match tie.visit() {
        Some(visit) => map(mapper, read, canonical, index, Some(visit)).await?,
        None => graph,
    };
    Ok((graph, Some(tie)))
}

/// Maps `canonical` with `visit`, checking that the graph names the
/// composition.
async fn map<M: Mapper>(
    mapper: &M,
    read: &Read<'_>,
    canonical: &CanonicalComposition,
    index: &WebTemplateIndex,
    visit: Option<&VisitKey>,
) -> Result<RecordGraph, Refusal> {
    let graph = mapper
        .map(&SourceComposition {
            source: read.source.clone(),
            composition: canonical,
            template: index,
            visit,
            context: read.context,
        })
        .await?;
    if graph.source() == read.source {
        Ok(graph)
    } else {
        Err(Refusal::new(
            "the mapper answered with the graph of another composition",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::source;
    use crate::etl::aql::CheckedQuery;
    use serde_json::json;

    /// The version every case reads.
    const VERSION: &str = "8849182c-82ad-4088-a07f-48ead4180515::ferrobridge.test::2";

    #[test]
    fn the_versioned_object_is_read_from_the_version_when_the_query_selects_none() {
        let query = CheckedQuery::compositions(
            "SELECT e/ehr_id/value AS ehr_id, v/uid/value AS version_uid, c AS composition \
             FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c ORDER BY v/uid/value",
        )
        .expect("the query checks");
        let read =
            source(&query, &[json!("ehr-1"), json!(VERSION), json!({})]).expect("the row reads");
        assert_eq!(
            "8849182c-82ad-4088-a07f-48ead4180515",
            read.versioned_object_uid().as_str()
        );
    }

    #[test]
    fn a_selected_versioned_object_of_another_version_is_refused() {
        let query = CheckedQuery::compositions(
            "SELECT e/ehr_id/value AS ehr_id, vo/uid/value AS versioned_object_uid, \
             v/uid/value AS version_uid, c AS composition \
             FROM EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS VERSION v CONTAINS COMPOSITION c \
             ORDER BY v/uid/value",
        )
        .expect("the query checks");
        let row = [json!("ehr-1"), json!("another"), json!(VERSION), json!({})];
        assert!(
            source(&query, &row).is_err(),
            "the version is no version of it"
        );
    }
}
