// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOP ETL run over the CDR: read, map, commit, then derive.
//!
//! The run reads the configured AQL page by page, maps each composition to
//! its record graph, commits it whole, and rebuilds the derived tables at the
//! end (`docs/architecture.md` §5.1, §5.4).
//!
//! The composition query selects each composition whole beside its
//! `ehr_id`, `versioned_object_uid` and `version_uid`; the runner reads the
//! template the composition names from the CDR once per run, validates the
//! composition against it, and hands it to a [`Mapper`]. A composition the
//! mapper or the writer refuses is rolled back and reported, and the run goes
//! on; a CDR or database that stops answering ends the run with a typed
//! error. A resumed run skips every composition whose watermark names the
//! version the query answered. No specification governs the run: our own
//! design, over openEHR ITS-REST 1.1.0 `POST /query/aql` with `offset` and
//! `fetch` paging.

pub mod aql;
pub mod mapper;
pub mod report;
pub mod visits;

use crate::etl::aql::{CheckedQuery, SINCE_PARAMETER};
use crate::etl::report::RunReport;
use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::ids::ObjectVersionId;
use ferrobridge_openehr::query::{PageSize, QueryPageError};
use futures_util::StreamExt;
use omop_cdm::derived;
use omop_cdm::graph::{EhrId, RecordGraph, Refusal, Source, VersionUid, VersionedObjectUid};
use omop_cdm::writer::{CdmWriter, RunId, VisitConcepts, WriteError};
use openehr_its::rest::generated::query::AdhocQueryExecute;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;
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

/// Reads the composition version a row names, checking that its
/// `version_uid` belongs to its `versioned_object_uid` (RM Common,
/// `OBJECT_VERSION_ID`).
fn source(query: &CheckedQuery, row: &[serde_json::Value]) -> Result<Source, Refusal> {
    let ehr = text(query, row, "ehr_id")?;
    let versioned = text(query, row, "versioned_object_uid")?;
    let version = text(query, row, "version_uid")?;
    let parsed = ObjectVersionId::new(version)
        .map_err(|error| Refusal::new(format!("the version_uid `{version}`: {error}")))?;
    if parsed.versioned_object_uid().as_str() != versioned {
        return Err(Refusal::new(format!(
            "the version_uid `{version}` is no version of `{versioned}`"
        )));
    }
    let refused = |error: omop_cdm::graph::EmptyIdentifier| Refusal::new(error.to_string());
    Ok(Source::new(
        EhrId::new(ehr).map_err(refused)?,
        VersionedObjectUid::new(versioned).map_err(refused)?,
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
    let Some(template) = composition
        .pointer("/archetype_details/template_id/value")
        .and_then(serde_json::Value::as_str)
    else {
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
        let outcome =
            composition(cdr, &mut templates, query, &row, &source, context, mapper).await?;
        let graph = match outcome {
            Ok(graph) => graph,
            Err(refusal) => {
                report.composition_refused(Some(&source), &refusal);
                continue;
            }
        };
        match writer.commit(&graph, run_id).await {
            Ok(written) => report.committed(&source, &written),
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

/// Reads, validates and maps the composition of one row.
async fn composition<M: Mapper>(
    cdr: &Client,
    templates: &mut Templates,
    query: &CheckedQuery,
    row: &[serde_json::Value],
    source: &Source,
    context: MappingContext,
    mapper: &M,
) -> Result<Result<RecordGraph, Refusal>, RunError> {
    let value = match cell(query, row, "composition") {
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
    let graph = mapper
        .map(&SourceComposition {
            source: source.clone(),
            composition: &canonical,
            template: &index,
            context,
        })
        .await;
    Ok(match graph {
        Ok(graph) if graph.source() == source => Ok(graph),
        Ok(_) => Err(Refusal::new(
            "the mapper answered with the graph of another composition",
        )),
        Err(refusal) => Err(refusal),
    })
}
