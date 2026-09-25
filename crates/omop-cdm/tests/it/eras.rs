// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The PostgreSQL era scripts against the published scripts they translate.
//!
//! The CDM publishes the two scripts on its SQL scripts page
//! (<https://ohdsi.github.io/CommonDataModel/sqlScripts.html>), vendored as
//! `docs/specs/omop-cdm/site/sqlScripts.qmd`. Each test extracts one
//! published script and compares its digest with the one the translation in
//! `crates/omop-cdm/sql/` was made from, so a change upstream is a failing
//! build until the translation follows it.

use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt::Write;

/// The vendored source of the SQL scripts page.
const PAGE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/omop-cdm/site/sqlScripts.qmd"
);

/// Returns the lines of the first `sql` block after the heading `heading`,
/// each ending in a newline.
fn published(heading: &str) -> Result<String, Box<dyn Error>> {
    let page = std::fs::read_to_string(PAGE)?;
    let mut lines = page.lines().skip_while(|line| *line != heading);
    lines.next().ok_or("the heading is not on the page")?;
    let mut lines = lines.skip_while(|line| *line != "```sql");
    lines.next().ok_or("the heading has no sql block")?;
    let mut block = String::new();
    for line in lines.take_while(|line| *line != "```") {
        block.push_str(line);
        block.push('\n');
    }
    Ok(block)
}

/// Returns the lowercase hex SHA-256 of `text`.
fn digest(text: &str) -> Result<String, Box<dyn Error>> {
    let mut hex = String::new();
    for byte in Sha256::digest(text.as_bytes()) {
        write!(hex, "{byte:02x}")?;
    }
    Ok(hex)
}

#[test]
fn the_condition_era_translation_follows_the_published_script() -> Result<(), Box<dyn Error>> {
    let script = published("### Condition Eras")?;
    assert!(
        script.contains("INSERT INTO @TARGET_CDMV5_SCHEMA.condition_era ("),
        "the extracted block is not the condition era script"
    );
    assert_eq!(
        "0d669f910f7aae5dd972e51d138f2e34c22004a355e7cc4066e2aadfc98cdb2a",
        digest(&script)?,
        "the published Condition Eras script changed; translate crates/omop-cdm/sql/condition_era.sql again"
    );
    Ok(())
}

#[test]
fn the_drug_era_translation_follows_the_published_script() -> Result<(), Box<dyn Error>> {
    let script = published("### Drug Eras")?;
    assert!(
        script.contains("INSERT INTO @cdm_schema.drug_era("),
        "the extracted block is not the drug era script"
    );
    assert_eq!(
        "424a9437fb1aff71133fa4bc4c20b89163af45bc506223ee2c2a9b44a16fafc3",
        digest(&script)?,
        "the published Drug Eras script changed; translate crates/omop-cdm/sql/drug_era.sql again"
    );
    Ok(())
}
