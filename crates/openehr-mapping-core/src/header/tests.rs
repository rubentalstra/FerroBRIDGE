// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

use core::str::FromStr;

use super::archetype::ArchetypeId;
use super::archetype::ArchetypeIdError;
use super::grammar::GrammarSemVer;
use super::grammar::GrammarVersion;
use super::grammar::GrammarVersionError;
use super::grammar::MappingLanguage;
use super::metadata::MappingName;
use super::metadata::MappingNameError;
use super::metadata::MappingType;

#[test]
fn the_two_grammar_languages_parse() {
    let fhir = GrammarVersion::from_str("FHIRConnect/v1.0.0").expect("a FHIRconnect grammar");
    assert_eq!(fhir.language(), MappingLanguage::FhirConnect);
    assert_eq!(fhir.version(), GrammarSemVer::new(1, 0, 0));
    let omocl = GrammarVersion::from_str("OMOCL/v1.0.0").expect("an OMOCL grammar");
    assert_eq!(omocl.language(), MappingLanguage::Omocl);
}

#[test]
fn the_language_spelling_is_case_insensitive_and_kept_verbatim() {
    let schema_case = GrammarVersion::from_str("FHIRConnect/v1.0.0").expect("schema spelling");
    let prose_case = GrammarVersion::from_str("FHIRconnect/v1.0.0").expect("prose spelling");
    assert_eq!(schema_case.version(), prose_case.version());
    assert_eq!(schema_case.language(), prose_case.language());
    assert_eq!(prose_case.spelling(), "FHIRconnect");
    assert_eq!(prose_case.to_string(), "FHIRconnect/v1.0.0");
}

#[test]
fn a_two_part_version_and_an_unknown_language_are_refused() {
    assert_eq!(
        GrammarVersion::from_str("FHIRConnect/1.0"),
        Err(GrammarVersionError::MalformedVersion {
            version: "1.0".to_owned()
        })
    );
    assert_eq!(
        GrammarVersion::from_str("Foo/v1.0.0"),
        Err(GrammarVersionError::UnknownLanguage {
            language: "Foo".to_owned()
        })
    );
    assert_eq!(
        GrammarVersion::from_str("OMOCL"),
        Err(GrammarVersionError::MissingSeparator {
            value: "OMOCL".to_owned()
        })
    );
}

#[test]
fn the_file_types_parse_case_exactly() {
    assert_eq!(MappingType::from_str("model"), Ok(MappingType::Model));
    assert_eq!(MappingType::from_str("context"), Ok(MappingType::Context));
    assert!(MappingType::from_str("Model").is_err());
}

#[test]
fn an_archetype_id_follows_the_base_lexical_form() {
    let id = ArchetypeId::from_str("openEHR-EHR-EVALUATION.problem_diagnosis.v1")
        .expect("a well-formed archetype id");
    assert_eq!(id.version_number(), 1);
    assert_eq!(id.as_str(), "openEHR-EHR-EVALUATION.problem_diagnosis.v1");
    assert!(ArchetypeId::from_str("openEHR-EHR-CLUSTER.imaging_exam-foetus.v1").is_ok());
    assert!(ArchetypeId::from_str("openEHR-EHR-ACTION.informed_consent.v0").is_ok());
}

#[test]
fn an_archetype_id_without_a_version_is_refused() {
    assert_eq!(
        ArchetypeId::from_str("openEHR-EHR-EVALUATION.problem_diagnosis"),
        Err(ArchetypeIdError::Shape {
            value: "openEHR-EHR-EVALUATION.problem_diagnosis".to_owned()
        })
    );
    assert_eq!(
        ArchetypeId::from_str("openEHR-EHR-EVALUATION.problem_diagnosis.1"),
        Err(ArchetypeIdError::VersionId {
            value: "1".to_owned()
        })
    );
    assert_eq!(
        ArchetypeId::from_str("openEHR-EHR-EVALUATION.problem_diagnosis.v01"),
        Err(ArchetypeIdError::VersionId {
            value: "v01".to_owned()
        })
    );
    assert_eq!(
        ArchetypeId::from_str("openEHR-EHR.problem_diagnosis.v1"),
        Err(ArchetypeIdError::QualifiedRmEntity {
            value: "openEHR-EHR".to_owned()
        })
    );
}

#[test]
fn a_mapping_name_is_a_referenceable_id() {
    assert_eq!(
        MappingName::new("  padded  "),
        Err(MappingNameError::Untrimmed {
            name: "  padded  ".to_owned()
        })
    );
    assert_eq!(MappingName::new(""), Err(MappingNameError::Empty));
    assert_eq!(
        MappingName::new("ACTION.procedure.v1")
            .expect("a well-formed name")
            .as_str(),
        "ACTION.procedure.v1"
    );
}
