// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `etl run` job over a resolved configuration: the OMOCL set read from
//! `[mappings] omocl`, the CDR client of `[cdr]`, the concept resolver and
//! the writer over `[cdm]`, then one run.
//!
//! The binary calls [`run`] and prints the report; a test calls it with a
//! configuration of its own, so both drive the same path. No specification
//! governs the job: our own design.

use crate::config::Settings;
use crate::etl::mapper::{CdmVocabulary, MapperError, OmoclMapper, read_set};
use crate::etl::report::RunReport;
use crate::etl::{RunError, RunOptions};
use omocl::engine::concept::VocabularyAliases;
use omop_cdm::database::{CdmPool, ConnectError};
use omop_cdm::graph::EmptyIdentifier;
use omop_cdm::vocabulary::ConceptResolver;
use omop_cdm::writer::{CdmWriter, RunId, WriteError};

/// Why `etl run` could not run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JobError {
    /// The configuration lacks a section the job needs.
    #[error("`etl run` needs {0}")]
    Missing(&'static str),
    /// The OMOCL mapping set does not load.
    #[error("reading the OMOCL mapping set")]
    Mappings(#[source] MapperError),
    /// The CDR client could not be built.
    #[error("building the CDR client the run reads through")]
    Client(#[source] ferrobridge_openehr::error::Error),
    /// The concept resolver could not connect to the CDM database.
    #[error("connecting the concept resolver to the CDM database")]
    Resolver(#[source] ConnectError),
    /// The writer could not connect to the CDM database.
    #[error("connecting the CDM writer")]
    Writer(#[source] WriteError),
    /// The run identifier could not be formed.
    #[error("forming the run identifier")]
    RunId(#[source] EmptyIdentifier),
    /// The run stopped.
    #[error("running the ETL")]
    Run(#[source] RunError),
}

/// Returns the section `etl run` needs and `settings` lacks, the first
/// missing first.
#[must_use]
pub fn missing_section(settings: &Settings) -> Option<&'static str> {
    if settings.etl.is_none() {
        Some("an [etl] section")
    } else if settings.cdm.is_none() {
        Some("a [cdm] section")
    } else if settings.cdr.is_none() {
        Some("a [cdr] section")
    } else if settings.omocl_directory.is_none() {
        Some("[mappings] omocl")
    } else {
        None
    }
}

/// Runs the OMOP ETL once over `settings` and returns its report.
///
/// The OMOCL set is read before any upstream is called, so a mapping that
/// does not load refuses the start. The concept resolver and the writer open
/// their own connections to the `[cdm]` database, over the TLS the
/// configuration settled; the two clients never share a pool. The run carries a fresh UUID as its identifier.
///
/// # Errors
///
/// Returns [`JobError::Missing`] naming the first section the configuration
/// lacks, [`JobError::Mappings`] when the OMOCL set does not load, the
/// connection variants when a client cannot be built or connected, and
/// [`JobError::Run`] when the run stops.
pub async fn run(settings: &Settings, options: &RunOptions) -> Result<RunReport, JobError> {
    let missing = || JobError::Missing(missing_section(settings).unwrap_or("its configuration"));
    let (Some(etl), Some(cdm), Some(cdr), Some(directory)) = (
        settings.etl.as_ref(),
        settings.cdm.as_ref(),
        settings.cdr.as_ref(),
        settings.omocl_directory.as_ref(),
    ) else {
        return Err(missing());
    };
    let set = read_set(directory).map_err(JobError::Mappings)?;
    let client = ferrobridge_openehr::client::Client::new(cdr.clone()).map_err(JobError::Client)?;
    let pool = CdmPool::connect(
        sqlx::postgres::PgPoolOptions::new().max_connections(2),
        cdm.connection.pool_options(),
        cdm.schema.clone(),
    )
    .await
    .map_err(JobError::Resolver)?;
    let mapper = OmoclMapper::new(
        set,
        CdmVocabulary::new(ConceptResolver::new(pool)),
        VocabularyAliases::default(),
    );
    let mut writer = CdmWriter::connect_with(
        &cdm.connection,
        cdm.schema.clone(),
        cdm.bridge_schema.clone(),
        cdm.person_policy,
    )
    .await
    .map_err(JobError::Writer)?;
    let run_id = RunId::new(uuid::Uuid::new_v4().to_string()).map_err(JobError::RunId)?;
    crate::etl::run(etl, options, &client, &mut writer, &mapper, &run_id)
        .await
        .map_err(JobError::Run)
}
