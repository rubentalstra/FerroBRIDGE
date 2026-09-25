// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every refusal of the model layer, each on its own synthetic file, asserted
//! by code and by the file, line and column the diagnostic names.

use std::error::Error;

use omocl::model::ast::Alternative;
use omocl::model::ast::Entity;
use omocl::model::ast::Factor;
use omocl::model::error::ModelCode;
use omocl::model::load::load_set;
use omocl::model::load::load_str;
use omocl::model::semantic::ConverterRegistry;
use omocl::model::semantic::FirstPartyConverters;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::DiagnosticCode;

/// The header every synthetic file opens with, through line 11.
const HEADER: &str = "grammar: OMOCL/v1.0.0\ntype: model\nmetadata:\n  name: Test_v1\n  \
                      version: 1.0.0\nspec:\n  system: OMOP\n  version: 5.4\n  \
                      openEhrConfig:\n    archetype: openEHR-EHR-OBSERVATION.test.v1\n\
                      mappings:\n";

/// One valid `Measurement` record, lines 12 to 18.
const RECORD: &str = "  - type: \"Measurement\"\n    concept_id:\n      alternatives:\n        \
                      - code: 3004249\n    measurement_date:\n      alternatives:\n        \
                      - path: \"../context\"\n";

/// Loads `source` as `test.yml` and returns its diagnostics.
fn refusals(source: &str) -> Vec<Diagnostic> {
    match load_str("test.yml", source, &FirstPartyConverters) {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics,
    }
}

/// Asserts `source` is refused first under `code` at `line:column`, and
/// returns the rendered first diagnostic.
fn refused_at(
    source: &str,
    code: &DiagnosticCode,
    line: u64,
    column: u64,
) -> Result<String, Box<dyn Error>> {
    let diagnostics = refusals(source);
    let first = diagnostics.first().ok_or("the file loaded")?;
    assert_eq!(first.code(), code, "{first}");
    assert_eq!(
        first.file().to_string_lossy(),
        "test.yml",
        "the diagnostic names the file"
    );
    let position = first.position().ok_or("the diagnostic names a position")?;
    assert_eq!(
        (position.line(), position.column()),
        (line, column),
        "{first}"
    );
    Ok(first.to_string())
}

#[test]
fn a_well_formed_file_loads() {
    let diagnostics = refusals(&format!("{HEADER}{RECORD}"));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn an_unknown_column_key_is_refused_naming_file_position_and_key() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}    colour:\n      alternatives:\n        - code: 1\n");
    let rendered = refused_at(&source, &ModelCode::UnknownKey.into(), 19, 5)?;
    assert!(
        rendered.starts_with("test.yml:19:5: error[omocl-unknown-key]"),
        "{rendered}"
    );
    assert!(rendered.contains("`colour`"), "{rendered}");
    assert!(rendered.contains("mappings[0].colour"), "{rendered}");
    Ok(())
}

#[test]
fn a_key_of_another_target_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    observation_date:\n      alternatives:\n        - path: \"/x\"\n"
    );
    refused_at(&source, &ModelCode::UnknownKey.into(), 19, 5)?;
    Ok(())
}

#[test]
fn an_unknown_root_key_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("engine: EOS/v0.0.62\n{HEADER}{RECORD}");
    refused_at(&source, &ModelCode::UnknownKey.into(), 1, 1)?;
    Ok(())
}

#[test]
fn a_measurement_qualifier_is_refused_with_its_reason() -> Result<(), Box<dyn Error>> {
    let source =
        format!("{HEADER}{RECORD}    qualifier:\n      alternatives:\n        - code: 4118850\n");
    let rendered = refused_at(&source, &ModelCode::KeyWithoutColumn.into(), 19, 5)?;
    assert!(rendered.contains("no qualifier column"), "{rendered}");
    Ok(())
}

#[test]
fn a_column_without_alternatives_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}    unit:\n      optional: true\n");
    refused_at(&source, &ModelCode::MissingKey.into(), 20, 7)?;
    Ok(())
}

#[test]
fn an_empty_alternatives_list_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}    unit:\n      alternatives: []\n");
    refused_at(&source, &ModelCode::EmptyList.into(), 20, 21)?;
    Ok(())
}

#[test]
fn an_alternative_with_two_forms_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    unit:\n      alternatives:\n        - path: \"/x\"\n          code: 1\n"
    );
    refused_at(&source, &ModelCode::AmbiguousAlternative.into(), 21, 11)?;
    Ok(())
}

#[test]
fn an_alternative_with_no_form_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}    unit:\n      alternatives:\n        - {{}}\n");
    refused_at(&source, &ModelCode::EmptyAlternative.into(), 21, 11)?;
    Ok(())
}

#[test]
fn an_unknown_alternative_form_is_refused() -> Result<(), Box<dyn Error>> {
    let source =
        format!("{HEADER}{RECORD}    unit:\n      alternatives:\n        - literal: \"mg\"\n");
    refused_at(&source, &ModelCode::UnknownKey.into(), 21, 11)?;
    Ok(())
}

#[test]
fn a_concept_id_outside_a_cdm_integer_is_refused() -> Result<(), Box<dyn Error>> {
    let source =
        format!("{HEADER}{RECORD}    unit:\n      alternatives:\n        - code: 2147483648\n");
    refused_at(&source, &ModelCode::InvalidConceptId.into(), 21, 17)?;
    Ok(())
}

#[test]
fn a_text_code_is_refused() -> Result<(), Box<dyn Error>> {
    let source =
        format!("{HEADER}{RECORD}    unit:\n      alternatives:\n        - code: \"mg\"\n");
    refused_at(&source, &ModelCode::UnexpectedNodeKind.into(), 21, 17)?;
    Ok(())
}

#[test]
fn a_concept_map_key_that_is_no_at_code_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    value:\n      alternatives:\n        - conceptMap:\n            \
         path: \"/x\"\n            mapping:\n              local::at0003: 4263218\n"
    );
    refused_at(&source, &ModelCode::InvalidAtCode.into(), 24, 15)?;
    Ok(())
}

#[test]
fn an_empty_concept_map_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    value:\n      alternatives:\n        - conceptMap:\n            \
         path: \"/x\"\n            mapping: {{}}\n"
    );
    refused_at(&source, &ModelCode::EmptyList.into(), 23, 22)?;
    Ok(())
}

// NOTE: the OMOCL railroad (`docs/specs/omocl/docs/wiki-images/omop_railroad.png`,
// CONVERSION clause) follows `multiplication` with CDM FIELD clauses, `code` among them.
#[test]
fn a_multiplication_of_a_path_and_a_code_loads() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    value:\n      alternatives:\n        - multiplication:\n            \
         - path: \"/items[at0001]\"\n            - code: 1000\n"
    );
    let file = load_str("test.yml", &source, &FirstPartyConverters)
        .map_err(|diagnostics| crate::support::render(&diagnostics))?;
    let Some(Entity::Record(record)) = file.entities.first() else {
        return Err("the file opens with a record".into());
    };
    let value = record
        .columns
        .iter()
        .find(|column| column.key() == "value")
        .ok_or("a value column")?;
    let [Alternative::Multiplication(product)] = value.alternatives.as_slice() else {
        return Err("the value is one multiplication".into());
    };
    let [Factor::Path(path), Factor::Code(code)] = product.factors.as_slice() else {
        return Err("a path factor and a code factor, in file order".into());
    };
    assert_eq!(
        path.value().to_string(),
        "/items[at0001]",
        "the path factor"
    );
    assert_eq!(*code.value(), 1000, "the code factor");
    Ok(())
}

#[test]
fn a_multiplication_factor_that_is_a_concept_map_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    value:\n      alternatives:\n        - multiplication:\n            \
         - conceptMap:\n                path: \"/x\"\n                mapping:\n                  at0001: 1\n"
    );
    refused_at(&source, &ModelCode::UnknownKey.into(), 22, 15)?;
    Ok(())
}

#[test]
fn a_multiplication_factor_with_a_path_and_a_code_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    value:\n      alternatives:\n        - multiplication:\n            \
         - path: \"/x\"\n              code: 2\n"
    );
    refused_at(&source, &ModelCode::AmbiguousAlternative.into(), 22, 15)?;
    Ok(())
}

#[test]
fn a_text_code_factor_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    value:\n      alternatives:\n        - multiplication:\n            \
         - code: \"1000\"\n"
    );
    refused_at(&source, &ModelCode::UnexpectedNodeKind.into(), 22, 21)?;
    Ok(())
}

#[test]
fn a_malformed_path_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    unit:\n      alternatives:\n        - path: \"/items/[at0012]\"\n"
    );
    refused_at(&source, &DiagnosticCode::MalformedPath, 21, 17)?;
    Ok(())
}

#[test]
fn optional_that_is_no_boolean_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    unit:\n      optional: \"yes\"\n      alternatives:\n        - path: \"/x\"\n"
    );
    refused_at(&source, &ModelCode::UnexpectedNodeKind.into(), 20, 17)?;
    Ok(())
}

#[test]
fn two_keys_writing_one_column_are_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}  - type: \"ProcedureOccurrence\"\n    procedure_date:\n      alternatives:\n        \
         - path: \"/a\"\n    procedure_start_date:\n      alternatives:\n        - path: \"/b\"\n"
    );
    let rendered = refused_at(&source, &ModelCode::ColumnClaimedTwice.into(), 16, 5)?;
    assert!(
        rendered.contains("procedure_occurrence.procedure_date"),
        "{rendered}"
    );
    Ok(())
}

#[test]
fn an_unknown_type_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}  - type: \"VisitOccurrence\"\n");
    refused_at(&source, &ModelCode::UnknownType.into(), 12, 11)?;
    Ok(())
}

#[test]
fn an_empty_mappings_list_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{} []\n", HEADER.trim_end_matches('\n'));
    refused_at(&source, &ModelCode::EmptyList.into(), 11, 11)?;
    Ok(())
}

#[test]
fn another_grammar_version_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}").replace("OMOCL/v1.0.0", "OMOCL/v1.0.1");
    refused_at(&source, &ModelCode::UnsupportedGrammar.into(), 1, 10)?;
    Ok(())
}

#[test]
fn a_lowercase_language_spelling_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}").replace("OMOCL/v1.0.0", "omocl/v1.0.0");
    refused_at(&source, &ModelCode::UnsupportedGrammar.into(), 1, 10)?;
    Ok(())
}

#[test]
fn a_context_file_type_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}").replace("type: model", "type: context");
    refused_at(&source, &ModelCode::UnsupportedFileType.into(), 2, 7)?;
    Ok(())
}

#[test]
fn a_file_without_a_header_type_loads() {
    let source = format!("{HEADER}{RECORD}").replace("type: model\n", "");
    let diagnostics = refusals(&source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn another_cdm_system_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}").replace("system: OMOP", "system: FHIR");
    refused_at(&source, &ModelCode::UnsupportedTarget.into(), 7, 11)?;
    Ok(())
}

#[test]
fn another_cdm_version_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}").replace("version: 5.4", "version: 5.3");
    refused_at(&source, &ModelCode::UnsupportedTarget.into(), 8, 12)?;
    Ok(())
}

#[test]
fn a_quoted_cdm_version_loads() {
    let source = format!("{HEADER}{RECORD}").replace("version: 5.4", "version: \"5.4\"");
    let diagnostics = refusals(&source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn a_missing_archetype_is_refused() {
    let source = format!("{HEADER}{RECORD}").replace(
        "  openEhrConfig:\n    archetype: openEHR-EHR-OBSERVATION.test.v1\n",
        "",
    );
    let diagnostics = refusals(&source);
    let codes: Vec<String> = diagnostics.iter().map(|d| d.code().to_string()).collect();
    assert_eq!(
        codes,
        ["omocl-missing-key", "omocl-missing-key"],
        "{diagnostics:?}"
    );
}

#[test]
fn a_spec_revision_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}{RECORD}").replace(
        "    archetype: openEHR-EHR-OBSERVATION.test.v1\n",
        "    archetype: openEHR-EHR-OBSERVATION.test.v1\n    revision: 1.0.0\n",
    );
    refused_at(&source, &ModelCode::UnknownKey.into(), 11, 5)?;
    Ok(())
}

#[test]
fn an_include_naming_no_archetype_id_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}  - type: \"Include\"\n    archetype_id: \"CLUSTER.x.v1\"\n");
    refused_at(&source, &DiagnosticCode::InvalidArchetypeId, 13, 19)?;
    Ok(())
}

#[test]
fn an_include_without_archetype_id_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}  - type: \"Include\"\n    base_path: \"/items\"\n");
    refused_at(&source, &ModelCode::MissingKey.into(), 12, 5)?;
    Ok(())
}

#[test]
fn an_include_with_a_column_key_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}  - type: \"Include\"\n    archetype_id: \"openEHR-EHR-CLUSTER.x.v1\"\n    \
         concept_id:\n      alternatives:\n        - code: 1\n"
    );
    refused_at(&source, &ModelCode::UnknownKey.into(), 14, 5)?;
    Ok(())
}

#[test]
fn an_unknown_converter_is_refused() -> Result<(), Box<dyn Error>> {
    let source = format!("{HEADER}  - type: \"CustomMapping\"\n    name: \"NoSuchConverter\"\n");
    refused_at(&source, &ModelCode::UnknownConverter.into(), 13, 11)?;
    Ok(())
}

#[test]
fn the_first_party_converter_loads() {
    let source = format!(
        "{HEADER}  - type: \"CustomMapping\"\n    name: \"FactRelationshipCustomConverter\"\n"
    );
    let diagnostics = refusals(&source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn a_registry_of_the_callers_admits_its_own_converter() {
    /// A registry holding one caller-defined converter.
    #[derive(Debug)]
    struct Own;
    impl ConverterRegistry for Own {
        fn contains(&self, name: &str) -> bool {
            name == "OwnConverter"
        }
    }
    let source = format!("{HEADER}  - type: \"CustomMapping\"\n    name: \"OwnConverter\"\n");
    assert!(load_str("test.yml", &source, &Own).is_ok());
}

#[test]
fn an_include_of_an_archetype_no_file_maps_is_refused() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let file = directory.path().join("Test_v1.yml");
    std::fs::write(
        &file,
        format!(
            "{HEADER}  - type: \"Include\"\n    archetype_id: \"openEHR-EHR-CLUSTER.absent.v1\"\n"
        ),
    )?;
    let diagnostics = load_set([&file], &FirstPartyConverters)
        .err()
        .ok_or("the set loaded")?;
    let first = diagnostics.first().ok_or("one diagnostic")?;
    assert_eq!(
        first.code(),
        &ModelCode::UnresolvedInclude.into(),
        "{first}"
    );
    let position = first.position().ok_or("a position")?;
    assert_eq!((position.line(), position.column()), (13, 19));
    Ok(())
}

#[test]
fn a_duplicate_yaml_key_is_refused_by_the_shared_loader() -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{HEADER}{RECORD}    value:\n      alternatives:\n        - conceptMap:\n            \
         path: \"/x\"\n            mapping:\n              at0005: 1\n              at0005: 2\n"
    );
    let diagnostics = refusals(&source);
    let first = diagnostics.first().ok_or("the file loaded")?;
    assert_eq!(first.code(), &DiagnosticCode::YamlDuplicateKey, "{first}");
    Ok(())
}
