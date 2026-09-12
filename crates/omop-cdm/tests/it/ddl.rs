// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The embedded DDL against the vendored files, and the schema substitution
//! against the identifier rule it enforces.

use std::error::Error;
use std::fs;

/// The vendored DDL directory the emitter copies into the crate.
const VENDORED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/omop-cdm/inst/ddl/5.4/postgresql/"
);

#[test]
fn the_embedded_ddl_is_the_vendored_text() -> Result<(), Box<dyn Error>> {
    for (embedded, file) in [
        (
            omop_cdm::generated::ddl::DDL,
            "OMOPCDM_postgresql_5.4_ddl.sql",
        ),
        (
            omop_cdm::generated::ddl::PRIMARY_KEYS,
            "OMOPCDM_postgresql_5.4_primary_keys.sql",
        ),
        (
            omop_cdm::generated::ddl::INDICES,
            "OMOPCDM_postgresql_5.4_indices.sql",
        ),
        (
            omop_cdm::generated::ddl::CONSTRAINTS,
            "OMOPCDM_postgresql_5.4_constraints.sql",
        ),
    ] {
        let vendored = fs::read_to_string(format!("{VENDORED}{file}"))?;
        assert_eq!(
            vendored, embedded,
            "the embedded {file} is not the vendored text"
        );
    }
    Ok(())
}

#[test]
fn the_schema_substitution_replaces_every_placeholder() -> Result<(), Box<dyn Error>> {
    let statements = omop_cdm::ddl::with_schema(omop_cdm::generated::ddl::DDL, "omop_cdm")?;
    assert!(
        !statements.contains(omop_cdm::ddl::SCHEMA_PLACEHOLDER),
        "a placeholder survived the substitution"
    );
    assert!(statements.contains("CREATE TABLE omop_cdm.person"));
    assert!(statements.contains("CREATE TABLE omop_cdm.concept"));
    Ok(())
}

#[test]
fn the_schema_substitution_refuses_anything_but_an_identifier() {
    for refused in [
        "public; drop",
        "public\"",
        "public.cdm",
        "Public",
        "1cdm",
        "cdm-v5",
        "",
        "omop cdm",
    ] {
        assert!(
            omop_cdm::ddl::with_schema(omop_cdm::generated::ddl::DDL, refused).is_err(),
            "`{refused}` was accepted as a schema name"
        );
    }
}

#[test]
fn a_schema_name_stops_at_the_postgresql_identifier_length() {
    let boundary = "c".repeat(omop_cdm::ddl::MAX_IDENTIFIER_BYTES);
    assert!(omop_cdm::ddl::SchemaName::new(boundary).is_ok());

    let over_long = "c".repeat(omop_cdm::ddl::MAX_IDENTIFIER_BYTES + 1);
    assert_eq!(
        Err(omop_cdm::ddl::SchemaNameError::TooLong {
            length: omop_cdm::ddl::MAX_IDENTIFIER_BYTES + 1
        }),
        omop_cdm::ddl::SchemaName::new(over_long)
    );
}
