// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[cdm]` and `[etl]` configuration: the schema names, the person
//! policy, and the two queries checked when the configuration loads.

use ferrobridge_server::config::{Config, Error};
use ferrobridge_server::etl::aql::AqlError;
use std::collections::BTreeMap;
use std::error::Error as StdError;

/// An `[etl]` section with every required key and a composition query that
/// carries the four projections and an order.
const ETL: &str = r#"
[etl]
aql = "SELECT e/ehr_id/value AS ehr_id, vo/uid/value AS versioned_object_uid, v/uid/value AS version_uid, c AS composition FROM EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS VERSION v CONTAINS COMPOSITION c ORDER BY v/uid/value"
type_concept_id = 32817
observation_period_type_concept_id = 32880
"#;

/// A visit query with the four projections.
const VISITS: &str = "SELECT e/ehr_id/value AS ehr_id, c/name/value AS visit_source, c/context/start_time/value AS visit_start, c/context/end_time/value AS visit_end FROM EHR e CONTAINS COMPOSITION c ORDER BY c/context/start_time/value";

/// Resolves `text`, returning the refusal.
fn refusal(text: &str) -> Result<Error, Box<dyn StdError>> {
    match Config::from_sources(Some(text), &BTreeMap::new())?.resolve() {
        Ok(_) => Err("the configuration resolved".into()),
        Err(error) => Ok(error),
    }
}

#[test]
fn the_cdm_section_defaults_its_schemas_and_policy() -> Result<(), Box<dyn StdError>> {
    let text = "[cdm]\nurl = \"postgres://db.invalid/cdm\"\n";
    let settings = Config::from_sources(Some(text), &BTreeMap::new())?.resolve()?;
    let cdm = settings.cdm.as_ref().ok_or("the CDM lane is on")?;
    assert_eq!("cdm", cdm.schema.as_str());
    assert_eq!("ferrobridge", cdm.bridge_schema.as_str());
    assert_eq!(
        omop_cdm::writer::PersonPolicy::CreateOnFirstSight,
        cdm.person_policy
    );
    Ok(())
}

#[test]
fn a_schema_that_is_no_identifier_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error =
        refusal("[cdm]\nurl = \"postgres://db.invalid/cdm\"\nbridge_schema = \"bridge; drop\"\n")?;
    match &error {
        Error::Schema { key, .. } => assert_eq!("cdm.bridge_schema", key),
        other => return Err(format!("expected a schema error, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn an_unknown_person_policy_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = refusal("[cdm]\nurl = \"postgres://db.invalid/cdm\"\nperson_policy = \"merge\"\n")?;
    assert!(matches!(error, Error::PersonPolicy { .. }), "{error:?}");
    Ok(())
}

#[test]
fn an_etl_section_resolves_with_its_query_checked() -> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(Some(ETL), &BTreeMap::new())?.resolve()?;
    let etl = settings.etl.as_ref().ok_or("the ETL is configured")?;
    assert_eq!(32817, etl.type_concept_id);
    assert_eq!(32880, etl.observation_period_type_concept_id);
    assert_eq!(Some(2), etl.compositions.column("version_uid"));
    assert_eq!(100, etl.page_size.rows(), "the default page");
    assert!(etl.visits.is_none());
    Ok(())
}

#[test]
fn a_composition_query_without_its_projections_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = refusal(&ETL.replace("AS versioned_object_uid", "AS object"))?;
    match &error {
        Error::Aql { key, source } => {
            assert_eq!("etl.aql", key);
            assert!(
                matches!(
                    **source,
                    AqlError::Missing {
                        projection: "versioned_object_uid",
                        ..
                    }
                ),
                "{source:?}"
            );
        }
        other => return Err(format!("expected an AQL refusal, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn a_composition_query_without_an_order_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = refusal(&ETL.replace(" ORDER BY v/uid/value", ""))?;
    match &error {
        Error::Aql { source, .. } => {
            assert!(matches!(**source, AqlError::Unordered { .. }), "{source:?}");
        }
        other => return Err(format!("expected an AQL refusal, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn a_visit_query_that_does_not_parse_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = refusal(&format!(
        "{ETL}\n[etl.visits]\naql = \"SELECT FROM\"\nvisit_concept_id = 9201\nvisit_type_concept_id = 32817\n"
    ))?;
    match &error {
        Error::Aql { key, source } => {
            assert_eq!("etl.visits.aql", key);
            assert!(matches!(**source, AqlError::Parse { .. }), "{source:?}");
        }
        other => return Err(format!("expected an AQL refusal, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn a_visit_query_without_its_four_projections_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    for (alias, projection) in [
        ("AS ehr_id", "ehr_id"),
        ("AS visit_source", "visit_source"),
        ("AS visit_start", "visit_start"),
        ("AS visit_end", "visit_end"),
    ] {
        let query = VISITS.replace(alias, "AS other");
        let error = refusal(&format!(
            "{ETL}\n[etl.visits]\naql = \"{query}\"\nvisit_concept_id = 9201\nvisit_type_concept_id = 32817\n"
        ))?;
        match &error {
            Error::Aql { source, .. } => assert!(
                matches!(**source, AqlError::Missing { projection: missing, .. } if missing == projection),
                "{projection}: {source:?}"
            ),
            other => return Err(format!("expected an AQL refusal, got {other:?}").into()),
        }
    }
    Ok(())
}

#[test]
fn a_visit_section_without_its_concepts_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = refusal(&format!("{ETL}\n[etl.visits]\naql = \"{VISITS}\"\n"))?;
    match &error {
        Error::Missing { key } => assert_eq!("etl.visits.visit_concept_id", key),
        other => return Err(format!("expected a missing key, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn an_etl_section_without_a_type_concept_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = refusal(&ETL.replace("\ntype_concept_id = 32817", ""))?;
    match &error {
        Error::Missing { key } => assert_eq!("etl.type_concept_id", key),
        other => return Err(format!("expected a missing key, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn a_page_size_of_zero_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = refusal(&format!("{ETL}page_size = 0\n"))?;
    assert!(matches!(error, Error::PageSize { .. }), "{error:?}");
    Ok(())
}
