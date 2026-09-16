// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A malformed file is refused with a diagnostic that names the file and the
//! place in it.
//!
//! The fixtures beside this module are synthetic content written for these
//! tests (`.claude/rules/testing.md`), each malformed in exactly one way.

use std::error::Error;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::diagnostic::Severity;
use openehr_mapping_core::loader;
use openehr_mapping_core::position::Position;

/// A fixture beside this test module.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")).join(name)
}

/// Loads a fixture and returns the diagnostic it is refused with.
fn refusal(name: &str) -> Result<Diagnostic, Box<dyn Error>> {
    let file = fixture(name);
    match loader::load_file(file.clone()) {
        Ok(_) => Err(format!("{} was expected to be refused", file.display()).into()),
        Err(error) => Ok(Diagnostic::from(&error)),
    }
}

#[test]
fn a_tab_indented_file_is_refused_at_the_tab() -> Result<(), Box<dyn Error>> {
    let diagnostic = refusal("tab-indented.yml")?;
    assert_eq!(diagnostic.severity(), Severity::Error);
    assert_eq!(diagnostic.code(), &DiagnosticCode::YamlSyntax);
    assert_eq!(diagnostic.position(), Some(Position::new(4, 2)));
    assert_eq!(diagnostic.file(), fixture("tab-indented.yml"));
    Ok(())
}

#[test]
fn an_unresolvable_alias_is_refused_at_the_alias() -> Result<(), Box<dyn Error>> {
    let diagnostic = refusal("unresolvable-alias.yml")?;
    assert_eq!(diagnostic.code(), &DiagnosticCode::YamlUnresolvedAlias);
    assert_eq!(diagnostic.position(), Some(Position::new(13, 12)));
    Ok(())
}

#[test]
fn a_duplicate_key_is_refused_at_the_second_key() -> Result<(), Box<dyn Error>> {
    let diagnostic = refusal("duplicate-key.yml")?;
    assert_eq!(diagnostic.code(), &DiagnosticCode::YamlDuplicateKey);
    assert_eq!(diagnostic.position(), Some(Position::new(11, 3)));
    Ok(())
}

#[test]
fn an_archetype_id_without_a_version_is_refused_at_the_value() -> Result<(), Box<dyn Error>> {
    let diagnostic = refusal("archetype-without-version.yml")?;
    assert_eq!(diagnostic.code(), &DiagnosticCode::InvalidArchetypeId);
    assert_eq!(diagnostic.position(), Some(Position::new(10, 16)));
    assert_eq!(
        diagnostic.model_path().to_string(),
        "spec.openEhrConfig.archetype"
    );
    Ok(())
}

#[test]
fn a_fhirconnect_file_without_a_type_is_refused() -> Result<(), Box<dyn Error>> {
    let source = "grammar: FHIRConnect/v1.0.0\nmetadata:\n  name: no.type\n  version: 0.0.1\n\
                  spec:\n  system: FHIR\n  version: R4\n";
    let error = loader::load_str("no-type.yml", source)
        .err()
        .ok_or("a FHIRconnect file without `type` must be refused")?;
    assert_eq!(Diagnostic::from(&error).code(), &DiagnosticCode::MissingKey);
    Ok(())
}

#[test]
fn an_omocl_file_without_a_type_loads() -> Result<(), Box<dyn Error>> {
    let source = "grammar: OMOCL/v1.0.0\nmetadata:\n  name: no.type\n  version: 1.0.0\n\
                  spec:\n  system: OMOP\n  version: 5.4\n";
    let document = loader::load_str("no-type.yml", source)?;
    assert!(document.header().mapping_type().is_none());
    Ok(())
}
