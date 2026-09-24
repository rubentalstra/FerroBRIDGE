// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The synthetic fixtures the suites commit and map.
//!
//! Every byte here is invented for the tests: no clinical content, no real
//! identifier, no extract from any system. Identifier and code systems sit
//! under `http://example.org`, which RFC 6761 §6.5 reserves for documentation
//! and examples, except where a FHIR required binding fixes the system.
//!
//! Two fixtures name systems outside that domain. [`KDS_DIAGNOSE_OPT`] is a
//! published operational template vendored verbatim with its provenance beside
//! it: a template carries constraints, never patient content.
//! [`KDS_DIAGNOSE_CONDITION`] is synthetic, and names the profile and the
//! systems and extension URLs the published KDS mappings match on, because a
//! mapping condition only admits the value it names.

/// A synthetic operational template: one `EVALUATION` with one `DV_TEXT`
/// element under an `ITEM_TREE`.
///
/// The template identifier is `ferrobridge.minimal_evaluation.v1`.
pub const MINIMAL_EVALUATION_OPT: &str = include_str!("../fixtures/opt/minimal_evaluation.opt");

/// The template identifier [`MINIMAL_EVALUATION_OPT`] declares.
pub const MINIMAL_EVALUATION_TEMPLATE_ID: &str = "ferrobridge.minimal_evaluation.v1";

/// A synthetic operational template shaped like a diagnosis mapping chain.
///
/// One `COMPOSITION` holds one `EVALUATION` with the at-codes and the two
/// slotted `CLUSTER`s the diagnosis mappings of the FHIRconnect mapping
/// library write. The template identifier is `ferrobridge.diagnose.v1`.
pub const DIAGNOSE_OPT: &str = include_str!("../fixtures/opt/diagnose.opt");

/// The template identifier [`DIAGNOSE_OPT`] declares.
pub const DIAGNOSE_TEMPLATE_ID: &str = "ferrobridge.diagnose.v1";

/// The published `KDS_Diagnose` operational template, vendored verbatim.
///
/// One `COMPOSITION.report.v1` holding `EVALUATION.problem_diagnosis.v1` with
/// its slotted clusters, the template the FHIR round trip runs against.
/// `scripts/vendor/kds-diagnose-opt.sh` fetches it at the commit
/// `docs/VERSIONS.md` pins, and `PROVENANCE.md` beside it records its source
/// and licence.
pub const KDS_DIAGNOSE_OPT: &str = include_str!("../fixtures/opt/kds/KDS_Diagnose.opt");

/// The template identifier [`KDS_DIAGNOSE_OPT`] declares.
pub const KDS_DIAGNOSE_TEMPLATE_ID: &str = "KDS_Diagnose";

/// A synthetic canonical COMPOSITION built against
/// [`MINIMAL_EVALUATION_OPT`].
pub const MINIMAL_EVALUATION_COMPOSITION: &str =
    include_str!("../fixtures/composition/minimal_evaluation.json");

/// The `DV_TEXT` value the one element of [`MINIMAL_EVALUATION_COMPOSITION`]
/// carries.
pub const MINIMAL_EVALUATION_NOTE: &str =
    "A synthetic note committed by the FerroBRIDGE end-to-end test";

/// A synthetic FHIR R4 `CodeSystem` with two concepts.
pub const TERM_SOURCE_CODE_SYSTEM: &str =
    include_str!("../fixtures/terminology/codesystem-source.json");

/// A synthetic FHIR R4 `CodeSystem` the concept map translates into.
pub const TERM_TARGET_CODE_SYSTEM: &str =
    include_str!("../fixtures/terminology/codesystem-target.json");

/// A synthetic FHIR R4 `ValueSet` holding one of the two source concepts.
pub const TERM_VALUE_SET: &str = include_str!("../fixtures/terminology/valueset-members.json");

/// A synthetic FHIR R4 `ConceptMap` from the source system to the target one.
pub const TERM_CONCEPT_MAP: &str =
    include_str!("../fixtures/terminology/conceptmap-source-to-target.json");

/// The canonical URL of [`TERM_SOURCE_CODE_SYSTEM`].
pub const TERM_SOURCE_SYSTEM: &str = "http://example.org/fhir/CodeSystem/ferrobridge-source";

/// The canonical URL of [`TERM_TARGET_CODE_SYSTEM`].
pub const TERM_TARGET_SYSTEM: &str = "http://example.org/fhir/CodeSystem/ferrobridge-target";

/// The canonical URL of [`TERM_VALUE_SET`].
pub const TERM_VALUE_SET_URL: &str = "http://example.org/fhir/ValueSet/ferrobridge-members";

/// The canonical URL of [`TERM_CONCEPT_MAP`].
pub const TERM_CONCEPT_MAP_URL: &str =
    "http://example.org/fhir/ConceptMap/ferrobridge-source-to-target";

/// The source concept that is a value-set member and maps `equivalent`.
pub const TERM_MEMBER_CODE: &str = "alpha";

/// The display of [`TERM_MEMBER_CODE`].
pub const TERM_MEMBER_DISPLAY: &str = "Alpha finding";

/// The target code [`TERM_MEMBER_CODE`] maps to.
pub const TERM_MEMBER_TARGET_CODE: &str = "A1";

/// The source concept that is not a value-set member and maps `wider`.
pub const TERM_NON_MEMBER_CODE: &str = "beta";

/// A code no fixture defines, for the not-found cases.
pub const TERM_UNKNOWN_CODE: &str = "gamma";

/// A synthetic FHIR R4 `Condition`.
pub const R4_CONDITION: &str = include_str!("../fixtures/fhir/r4/condition.json");

/// A synthetic FHIR R4 `Condition` shaped like the MII Diagnose profile.
///
/// It claims the profile the `KDS_Diagnose` mappings compile against, codes
/// its diagnosis in the ICD-10-GM system with the Diagnosesicherheit
/// extension, dates its onset with a period whose start carries the
/// lebensphase extension, and carries the asserted-date extension, the two
/// status elements, a body site and a note. Every code and every other system
/// is invented under `http://example.org`.
pub const KDS_DIAGNOSE_CONDITION: &str =
    include_str!("../fixtures/fhir/r4/kds_diagnose_condition.json");

/// A synthetic FHIR R4 `Observation`.
pub const R4_OBSERVATION: &str = include_str!("../fixtures/fhir/r4/observation.json");

#[cfg(test)]
mod tests {
    use super::{
        MINIMAL_EVALUATION_COMPOSITION, MINIMAL_EVALUATION_NOTE, MINIMAL_EVALUATION_OPT,
        MINIMAL_EVALUATION_TEMPLATE_ID, R4_CONDITION, R4_OBSERVATION,
    };

    /// The code systems the R4 required bindings of the two `Condition`
    /// status elements admit.
    ///
    /// `Condition.clinicalStatus` and `Condition.verificationStatus` bind at
    /// strength `required` to `ValueSet/condition-clinical` and
    /// `ValueSet/condition-ver-status` (`StructureDefinition-Condition`, R4),
    /// and each value set includes exactly the system below.
    const REQUIRED_BINDING_SYSTEMS: [&str; 2] = [
        "http://terminology.hl7.org/CodeSystem/condition-clinical",
        "http://terminology.hl7.org/CodeSystem/condition-ver-status",
    ];

    #[test]
    fn the_kds_condition_names_only_invented_codes_outside_the_matched_systems() {
        use super::KDS_DIAGNOSE_CONDITION;
        let matched = [
            "www.medizininformatik-initiative.de/fhir/core/modul-diagnose/StructureDefinition/Diagnose",
            "hl7.org/fhir/StructureDefinition/condition-assertedDate",
            "fhir.de/StructureDefinition/icd-10-gm-diagnosesicherheit",
            "fhir.de/CodeSystem/bfarm/icd-10-gm",
            "fhir.de/StructureDefinition/lebensphase",
        ];
        let bound: Vec<&str> = REQUIRED_BINDING_SYSTEMS
            .iter()
            .map(|system| system.trim_start_matches("http://"))
            .collect();
        for scheme in ["http://", "https://"] {
            for url in KDS_DIAGNOSE_CONDITION.split(scheme).skip(1) {
                assert!(
                    url.starts_with("example.org/")
                        || matched.iter().any(|system| url.starts_with(system))
                        || bound.iter().any(|system| url.starts_with(system)),
                    "the KDS Condition names a URL outside the matched systems: {url}"
                );
            }
        }
    }

    #[test]
    fn the_kds_template_declares_its_identifier() {
        use super::{KDS_DIAGNOSE_OPT, KDS_DIAGNOSE_TEMPLATE_ID};
        let declared = format!("<template_id>\n    <value>{KDS_DIAGNOSE_TEMPLATE_ID}</value>");
        assert!(
            KDS_DIAGNOSE_OPT.contains(&declared),
            "the vendored KDS template does not declare {KDS_DIAGNOSE_TEMPLATE_ID}"
        );
    }

    #[test]
    fn the_composition_names_the_template_and_carries_the_note() {
        assert!(MINIMAL_EVALUATION_OPT.contains(MINIMAL_EVALUATION_TEMPLATE_ID));
        assert!(MINIMAL_EVALUATION_COMPOSITION.contains(MINIMAL_EVALUATION_TEMPLATE_ID));
        assert!(MINIMAL_EVALUATION_COMPOSITION.contains(MINIMAL_EVALUATION_NOTE));
    }

    #[test]
    fn the_terminology_fixtures_name_their_canonicals_and_codes() {
        use super::{
            TERM_CONCEPT_MAP, TERM_CONCEPT_MAP_URL, TERM_MEMBER_CODE, TERM_MEMBER_DISPLAY,
            TERM_MEMBER_TARGET_CODE, TERM_NON_MEMBER_CODE, TERM_SOURCE_CODE_SYSTEM,
            TERM_SOURCE_SYSTEM, TERM_TARGET_CODE_SYSTEM, TERM_TARGET_SYSTEM, TERM_UNKNOWN_CODE,
            TERM_VALUE_SET, TERM_VALUE_SET_URL,
        };
        assert!(TERM_SOURCE_CODE_SYSTEM.contains(TERM_SOURCE_SYSTEM));
        assert!(TERM_SOURCE_CODE_SYSTEM.contains(TERM_MEMBER_DISPLAY));
        assert!(TERM_TARGET_CODE_SYSTEM.contains(TERM_TARGET_SYSTEM));
        assert!(TERM_TARGET_CODE_SYSTEM.contains(TERM_MEMBER_TARGET_CODE));
        assert!(TERM_VALUE_SET.contains(TERM_VALUE_SET_URL));
        assert!(TERM_CONCEPT_MAP.contains(TERM_CONCEPT_MAP_URL));
        assert!(
            TERM_VALUE_SET.contains(TERM_MEMBER_CODE),
            "the member is not in the value set"
        );
        assert!(
            !TERM_VALUE_SET.contains(TERM_NON_MEMBER_CODE),
            "the non-member is in the value set"
        );
        for fixture in [
            TERM_SOURCE_CODE_SYSTEM,
            TERM_TARGET_CODE_SYSTEM,
            TERM_VALUE_SET,
            TERM_CONCEPT_MAP,
        ] {
            assert!(
                !fixture.contains(TERM_UNKNOWN_CODE),
                "a fixture defines the code the not-found cases rely on being absent"
            );
            for url in fixture.split("http://").skip(1) {
                assert!(
                    url.starts_with("example.org/"),
                    "a terminology fixture names a URL outside http://example.org"
                );
            }
        }
    }

    #[test]
    fn the_condition_names_the_required_binding_systems() {
        for system in REQUIRED_BINDING_SYSTEMS {
            assert!(
                R4_CONDITION.contains(system),
                "the synthetic Condition does not name {system}"
            );
        }
    }

    #[test]
    fn every_other_fhir_url_is_under_the_example_domain() {
        let bound: Vec<&str> = REQUIRED_BINDING_SYSTEMS
            .iter()
            .map(|system| system.trim_start_matches("http://"))
            .collect();
        for (name, resource) in [("Condition", R4_CONDITION), ("Observation", R4_OBSERVATION)] {
            for url in resource.split("http://").skip(1) {
                assert!(
                    url.starts_with("example.org/")
                        || bound.iter().any(|system| url.starts_with(system)),
                    "the synthetic {name} names a URL that is neither under \
                     http://example.org nor a system a required binding fixes"
                );
            }
        }
    }
}
