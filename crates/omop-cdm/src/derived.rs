// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The derived tables: `OBSERVATION_PERIOD`, `CONDITION_ERA` and `DRUG_ERA`,
//! each rebuilt whole from the clinical tables of the CDM schema.
//!
//! The CDM leaves the observation period to the ETL, with suggestions
//! (<https://ohdsi.github.io/CommonDataModel/ehrObsPeriods.html>); the one
//! built here spans the first to the last clinical event of each person, no
//! specification governs it: our own design. The two era tables run the
//! scripts the CDM publishes
//! (<https://ohdsi.github.io/CommonDataModel/sqlScripts.html>) in their
//! PostgreSQL form, [`CONDITION_ERA_SQL`] and [`DRUG_ERA_SQL`]. Each rebuild
//! runs in one transaction, so a refusal leaves the earlier table in place.

use crate::ddl::{self, SchemaName};
use crate::writer::{CdmWriter, Step, WriteError};

/// The PostgreSQL form of the CDM's "Condition Eras" script
/// (`sql/PROVENANCE.md`), with the `@cdmDatabaseSchema` placeholder.
pub const CONDITION_ERA_SQL: &str = include_str!("../sql/condition_era.sql");

/// The PostgreSQL form of the CDM's "Drug Eras" script
/// (`sql/PROVENANCE.md`), with the `@cdmDatabaseSchema` placeholder.
pub const DRUG_ERA_SQL: &str = include_str!("../sql/drug_era.sql");

/// The clinical events the observation period spans: each table with the
/// column of the event's start and of its end, when the table has one.
pub const OBSERVATION_EVENTS: [(&str, &str, Option<&str>); 10] = [
    (
        "condition_occurrence",
        "condition_start_date",
        Some("condition_end_date"),
    ),
    (
        "drug_exposure",
        "drug_exposure_start_date",
        Some("drug_exposure_end_date"),
    ),
    (
        "procedure_occurrence",
        "procedure_date",
        Some("procedure_end_date"),
    ),
    (
        "device_exposure",
        "device_exposure_start_date",
        Some("device_exposure_end_date"),
    ),
    ("measurement", "measurement_date", None),
    ("observation", "observation_date", None),
    ("specimen", "specimen_date", None),
    ("note", "note_date", None),
    ("death", "death_date", None),
    (
        "visit_occurrence",
        "visit_start_date",
        Some("visit_end_date"),
    ),
];

/// Returns the rows `table` holds now, inside the rebuild's transaction.
async fn count(
    transaction: &tokio_postgres::Transaction<'_>,
    cdm: &SchemaName,
    table: &'static str,
) -> Result<u64, WriteError> {
    let row = transaction
        .query_one(&format!("SELECT count(*) FROM {cdm}.{table}"), &[])
        .await
        .map_err(|source| WriteError::Database {
            step: Step::Derive(table),
            source,
        })?;
    let rows: i64 = row.try_get(0).map_err(|source| WriteError::Database {
        step: Step::Derive(table),
        source,
    })?;
    Ok(rows.unsigned_abs())
}

/// Rebuilds `table` from the statements of `script` and returns its rows.
async fn rebuild(
    writer: &mut CdmWriter,
    table: &'static str,
    script: &str,
    parameter: Option<i32>,
) -> Result<u64, WriteError> {
    let (client, cdm) = writer.parts();
    let refused = |source| WriteError::Database {
        step: Step::Derive(table),
        source,
    };
    let transaction = client
        .transaction()
        .await
        .map_err(|source| WriteError::Database {
            step: Step::Begin,
            source,
        })?;
    transaction
        .execute(&format!("DELETE FROM {cdm}.{table}"), &[])
        .await
        .map_err(refused)?;
    match parameter {
        Some(value) => {
            transaction
                .execute(&ddl::in_schema(script, cdm), &[&value])
                .await
                .map_err(refused)?;
        }
        None => transaction
            .batch_execute(&ddl::in_schema(script, cdm))
            .await
            .map_err(refused)?,
    }
    let rows = count(&transaction, cdm, table).await?;
    transaction
        .commit()
        .await
        .map_err(|source| WriteError::Database {
            step: Step::Commit,
            source,
        })?;
    Ok(rows)
}

/// Returns the statement that fills `observation_period` from
/// [`OBSERVATION_EVENTS`], with the period type concept as `$1`.
fn observation_period_sql() -> String {
    let events = OBSERVATION_EVENTS
        .iter()
        .map(|(table, start, end)| {
            let end = end.map_or_else(
                || String::from(*start),
                |end| format!("COALESCE({end}, {start})"),
            );
            format!(
                "SELECT person_id, {start} AS start_date, {end} AS end_date FROM @cdmDatabaseSchema.{table}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n  UNION ALL ");
    format!(
        "INSERT INTO @cdmDatabaseSchema.observation_period
            (observation_period_id, person_id, observation_period_start_date,
             observation_period_end_date, period_type_concept_id)
         SELECT row_number() OVER (ORDER BY person_id), person_id,
                min(start_date), max(end_date), $1::integer
         FROM (\n  {events}\n) AS event
         GROUP BY person_id"
    )
}

/// Rebuilds `OBSERVATION_PERIOD`: one period per person, from the first to
/// the last clinical event, typed `period_type_concept_id`.
///
/// # Errors
///
/// Returns [`WriteError::Database`] naming the table when PostgreSQL refuses
/// the rebuild; the earlier rows stay.
pub async fn observation_period(
    writer: &mut CdmWriter,
    period_type_concept_id: i32,
) -> Result<u64, WriteError> {
    rebuild(
        writer,
        "observation_period",
        &observation_period_sql(),
        Some(period_type_concept_id),
    )
    .await
}

/// Rebuilds `CONDITION_ERA` with [`CONDITION_ERA_SQL`].
///
/// # Errors
///
/// Returns [`WriteError::Database`] naming the table when PostgreSQL refuses
/// the rebuild; the earlier rows stay.
pub async fn condition_era(writer: &mut CdmWriter) -> Result<u64, WriteError> {
    rebuild(writer, "condition_era", CONDITION_ERA_SQL, None).await
}

/// Rebuilds `DRUG_ERA` with [`DRUG_ERA_SQL`].
///
/// # Errors
///
/// Returns [`WriteError::Database`] naming the table when PostgreSQL refuses
/// the rebuild; the earlier rows stay.
pub async fn drug_era(writer: &mut CdmWriter) -> Result<u64, WriteError> {
    rebuild(writer, "drug_era", DRUG_ERA_SQL, None).await
}
