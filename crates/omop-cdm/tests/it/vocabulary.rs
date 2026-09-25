// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The concept resolver against the synthetic vocabulary, loaded into a CDM
//! schema the embedded DDL builds.
//!
//! The rules are the CDM conventions
//! (<https://ohdsi.github.io/CommonDataModel/dataModelConventions.html>) and
//! the vocabulary field definitions of CDM v5.4. The container-backed tests
//! run only when `FERROBRIDGE_E2E=1` admits the harness.

use ferrobridge_testkit::containers::{self, Postgres};
use ferrobridge_testkit::vocabulary;
use omop_cdm::database::{self, CdmPool};
use omop_cdm::ddl::SchemaName;
use omop_cdm::generated::concept::Concept;
use omop_cdm::value::CdmDate;
use omop_cdm::vocabulary::{
    ConceptCode, ConceptId, ConceptResolver, Resolution, ResolveError, SourceKey, VocabularyId,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::error::Error;
use std::time::Duration;

/// The schema the tests build the CDM in.
const SCHEMA: &str = "cdm";

/// The record date every case resolves on unless it says otherwise.
const RECORD_DATE: &str = "2026-06-15";

/// A resolver over the loaded fixture, with the container that holds it.
struct Loaded {
    /// Kept alive for the test; dropping it stops the container.
    postgres: Postgres,
    resolver: ConceptResolver,
}

/// Starts PostgreSQL, builds the CDM schema and loads the synthetic
/// vocabulary into it.
async fn loaded() -> Result<Loaded, Box<dyn Error>> {
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres, SCHEMA).await?;
    database::init(&pool).await?;
    let mut connection = pool.pool().acquire().await?;
    vocabulary::load(&mut connection).await?;
    drop(connection);
    Ok(Loaded {
        postgres,
        resolver: ConceptResolver::new(pool),
    })
}

/// Opens a pool on `postgres` bound to `schema`.
async fn connect(postgres: &Postgres, schema: &str) -> Result<CdmPool, Box<dyn Error>> {
    let options: PgConnectOptions = postgres.url().parse()?;
    Ok(CdmPool::connect(
        PgPoolOptions::new().max_connections(2),
        options,
        SchemaName::new(schema)?,
    )
    .await?)
}

/// Returns the key `(vocabulary, code)`.
fn key(vocabulary: &str, code: &str) -> Result<SourceKey, Box<dyn Error>> {
    Ok(SourceKey::new(
        VocabularyId::new(vocabulary)?,
        ConceptCode::new(code)?,
    ))
}

/// Returns the ids of `concepts`, in order.
fn ids(concepts: &[Concept]) -> Vec<i32> {
    concepts.iter().map(|concept| concept.concept_id).collect()
}

/// Returns the source concept's id and the standard concepts' ids.
fn shape(resolution: &Resolution) -> (Option<i32>, Vec<i32>) {
    (
        resolution
            .source_concept()
            .map(|concept| concept.concept_id),
        ids(resolution.standard_concepts()),
    )
}

#[tokio::test]
async fn a_standard_concept_resolves_to_itself() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(
            &key("FB-STANDARD", "STD-MEAS")?,
            &CdmDate::new(RECORD_DATE)?,
        )
        .await?;
    let Resolution::Standard { concept } = &resolution else {
        panic!("a standard concept is its own resolution: {resolution:?}");
    };
    assert_eq!(1001, concept.concept_id, "the wrong concept came back");
    assert_eq!(
        "Measurement",
        concept.domain_id.as_str(),
        "the domain_id does not travel with the concept"
    );
    Ok(())
}

#[tokio::test]
async fn a_source_concept_follows_one_maps_to_hop() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(&key("FB-SOURCE", "SRC-ONE")?, &CdmDate::new(RECORD_DATE)?)
        .await?;
    let Resolution::Mapped { source, standard } = &resolution else {
        panic!("a mapped source concept resolves through its Maps to: {resolution:?}");
    };
    assert_eq!(2001, source.concept_id, "the source concept is not kept");
    assert_eq!(vec![1002], ids(standard), "the Maps to target is wrong");
    assert_eq!(
        Some("Condition"),
        standard.first().map(|concept| concept.domain_id.as_str()),
        "the target's domain_id does not travel with it"
    );
    Ok(())
}

/// CDM conventions, "Mapping": one source concept may map to several standard
/// concepts.
#[tokio::test]
async fn a_fan_out_returns_every_standard_concept_in_id_order() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(&key("FB-SOURCE", "SRC-FAN")?, &CdmDate::new(RECORD_DATE)?)
        .await?;
    assert_eq!(
        (Some(2002), vec![1003, 1004]),
        shape(&resolution),
        "the fan-out is incomplete or out of concept_id order"
    );
    Ok(())
}

#[tokio::test]
async fn the_same_lookup_twice_yields_identical_rows() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let key = key("FB-SOURCE", "SRC-FAN")?;
    let date = CdmDate::new(RECORD_DATE)?;
    let first = loaded.resolver.resolve(&key, &date).await?;
    let second = loaded.resolver.resolve(&key, &date).await?;
    assert_eq!(first, second, "two lookups of one key disagree");
    Ok(())
}

/// CDM v5.4 `concept.invalid_reason`: `D` marks a deleted concept.
#[tokio::test]
async fn an_invalid_source_concept_is_excluded() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(
            &key("FB-SOURCE", "SRC-DELETED")?,
            &CdmDate::new(RECORD_DATE)?,
        )
        .await?;
    assert_eq!(
        (None, vec![]),
        shape(&resolution),
        "a deleted concept was used"
    );
    Ok(())
}

/// CDM v5.4 `concept_relationship.invalid_reason` and `concept.invalid_reason`:
/// the relationship and its target must both be valid.
#[tokio::test]
async fn an_invalid_relationship_or_target_is_excluded() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(&key("FB-SOURCE", "SRC-STALE")?, &CdmDate::new(RECORD_DATE)?)
        .await?;
    assert_eq!(
        (Some(2004), vec![1007]),
        shape(&resolution),
        "a deleted relationship or an updated target was followed"
    );
    Ok(())
}

/// CDM v5.4 `concept.valid_start_date`: a concept is not valid before it.
#[tokio::test]
async fn a_concept_not_yet_valid_is_excluded() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let key = key("FB-STANDARD", "STD-FUTURE")?;
    let before = loaded
        .resolver
        .resolve(&key, &CdmDate::new(RECORD_DATE)?)
        .await?;
    assert_eq!(
        (None, vec![]),
        shape(&before),
        "a concept was used before its valid_start_date"
    );
    let after = loaded
        .resolver
        .resolve(&key, &CdmDate::new("2031-01-01")?)
        .await?;
    assert_eq!(
        (Some(1008), vec![1008]),
        shape(&after),
        "the concept is refused inside its validity"
    );
    Ok(())
}

/// CDM v5.4 `concept.valid_end_date`: a concept is not valid after it.
#[tokio::test]
async fn a_concept_no_longer_valid_is_excluded() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(
            &key("FB-STANDARD", "STD-PAST")?,
            &CdmDate::new(RECORD_DATE)?,
        )
        .await?;
    assert_eq!(
        (None, vec![]),
        shape(&resolution),
        "a concept was used after its valid_end_date"
    );
    Ok(())
}

/// CDM v5.4 `concept_relationship.valid_start_date`: the relationship must be
/// valid on the record date too.
#[tokio::test]
async fn a_relationship_not_yet_valid_is_excluded() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(&key("FB-SOURCE", "SRC-LATER")?, &CdmDate::new(RECORD_DATE)?)
        .await?;
    let Resolution::Unmapped { source, .. } = &resolution else {
        panic!("a relationship valid from 2030 was followed: {resolution:?}");
    };
    assert_eq!(
        Some(2005),
        source.as_ref().map(|concept| concept.concept_id),
        "the valid source concept is not kept"
    );
    Ok(())
}

#[tokio::test]
async fn two_valid_concepts_under_one_key_are_an_error() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let refusal = loaded
        .resolver
        .resolve(&key("FB-SOURCE", "SRC-TWIN")?, &CdmDate::new(RECORD_DATE)?)
        .await
        .expect_err("an ambiguous key must never resolve to its first row");
    let ResolveError::Ambiguous { concept_ids, .. } = &refusal else {
        panic!("the refusal is not the ambiguity: {refusal}");
    };
    assert_eq!(
        &vec![ConceptId::new(2006), ConceptId::new(2007)],
        concept_ids,
        "the refusal does not list both concepts in order"
    );
    let message = refusal.to_string();
    assert!(
        message.contains("vocabulary_id 'FB-SOURCE', concept_code 'SRC-TWIN'"),
        "the refusal does not name the key: {message}"
    );
    assert!(
        message.contains(RECORD_DATE),
        "the refusal does not name the date: {message}"
    );
    Ok(())
}

#[tokio::test]
async fn the_validity_filters_run_before_the_ambiguity_check() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(
            &key("FB-SOURCE", "SRC-SUCCEEDED")?,
            &CdmDate::new(RECORD_DATE)?,
        )
        .await?;
    assert_eq!(
        (Some(2009), vec![1002]),
        shape(&resolution),
        "an updated concept made its successor's key ambiguous"
    );
    Ok(())
}

/// CDM conventions, "Mapping": concept 0 stands where no standard concept
/// exists; writing it and counting it is the caller's.
#[tokio::test]
async fn an_unknown_code_is_unmapped_with_its_key() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let key = key("FB-SOURCE", "SRC-NONE")?;
    let resolution = loaded
        .resolver
        .resolve(&key, &CdmDate::new(RECORD_DATE)?)
        .await?;
    assert_eq!(
        Resolution::Unmapped {
            key: key.clone(),
            source: None,
        },
        resolution,
        "an unknown code did not come back unmapped with its key"
    );
    assert_eq!(0, ConceptId::NO_MATCHING_CONCEPT.get(), "concept 0 moved");
    Ok(())
}

#[tokio::test]
async fn a_source_concept_without_maps_to_is_unmapped() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let resolution = loaded
        .resolver
        .resolve(&key("FB-SOURCE", "SRC-NOMAP")?, &CdmDate::new(RECORD_DATE)?)
        .await?;
    assert_eq!(
        (Some(2010), vec![]),
        shape(&resolution),
        "a relationship other than Maps to was followed"
    );
    Ok(())
}

/// CDM v5.4 `measurement.unit_concept_id`: a unit maps to a standard concept
/// in the Unit domain, and units are case-sensitive.
#[tokio::test]
async fn a_unit_resolves_through_the_standard_path() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let date = CdmDate::new(RECORD_DATE)?;
    let resolution = loaded
        .resolver
        .resolve(&key("FB-UNIT", "U-SYNTH")?, &date)
        .await?;
    let Resolution::Standard { concept } = &resolution else {
        panic!("a standard unit is its own resolution: {resolution:?}");
    };
    assert_eq!("Unit", concept.domain_id.as_str(), "the unit's domain_id");
    assert_eq!(
        "Synthetic µ-unit",
        concept.concept_name.as_str(),
        "the multi-byte name did not survive the load"
    );
    let folded = loaded
        .resolver
        .resolve(&key("FB-UNIT", "u-synth")?, &date)
        .await?;
    assert_eq!(
        (None, vec![]),
        shape(&folded),
        "a unit code matched another case"
    );
    Ok(())
}

#[tokio::test]
async fn a_query_the_database_refuses_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let loaded = loaded().await?;
    let empty = connect(&loaded.postgres, "no_cdm_here").await?;
    let refusal = ConceptResolver::new(empty)
        .resolve(&key("FB-SOURCE", "SRC-ONE")?, &CdmDate::new(RECORD_DATE)?)
        .await
        .expect_err("a schema without vocabulary tables cannot answer");
    let ResolveError::Query { source, .. } = &refusal else {
        panic!("the refusal is not the database's: {refusal}");
    };
    assert_eq!(
        Some("42P01"),
        source
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        "the refusal is not PostgreSQL's undefined_table"
    );
    assert!(
        refusal.to_string().contains(RECORD_DATE),
        "the refusal does not name the date: {refusal}"
    );
    Ok(())
}

#[tokio::test]
async fn the_loader_writes_every_fixture_row() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres, SCHEMA).await?;
    database::init(&pool).await?;
    let mut connection = pool.pool().acquire().await?;
    let written = vocabulary::load(&mut connection).await?;
    let expected: usize = vocabulary::TABLES
        .iter()
        .map(|table| table.csv.lines().count().saturating_sub(1))
        .sum();
    assert_eq!(
        u64::try_from(expected)?,
        written,
        "the loader wrote a different number of rows than the fixture holds"
    );
    Ok(())
}

#[tokio::test]
async fn an_unreachable_database_is_a_typed_error() -> Result<(), Box<dyn Error>> {
    let options: PgConnectOptions = "postgres://nobody:nothing@127.0.0.1:1/none".parse()?;
    let refusal = CdmPool::connect(
        PgPoolOptions::new().acquire_timeout(Duration::from_secs(5)),
        options,
        SchemaName::new(SCHEMA)?,
    )
    .await
    .expect_err("nothing listens on port 1");
    assert_eq!(SCHEMA, refusal.schema().as_str(), "the wrong schema named");
    assert!(
        refusal.source().is_some(),
        "the refusal lost the database's error"
    );
    Ok(())
}

#[test]
fn a_key_longer_than_its_column_is_refused() {
    assert!(
        VocabularyId::new("X".repeat(21)).is_err(),
        "vocabulary_id is varchar(20)"
    );
    assert!(
        ConceptCode::new("X".repeat(51)).is_err(),
        "concept_code is varchar(50)"
    );
}
