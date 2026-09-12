// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The synthetic fixtures the suites commit and map.
//!
//! Every byte here is invented for the tests: no clinical content, no real
//! identifier, no extract from any system. Identifier and code systems sit
//! under `http://example.org`, which RFC 6761 §6.5 reserves for documentation
//! and examples, except where a FHIR required binding fixes the system.

/// A synthetic operational template: one `EVALUATION` with one `DV_TEXT`
/// element under an `ITEM_TREE`.
///
/// The template identifier is `ferrobridge.minimal_evaluation.v1`.
pub const MINIMAL_EVALUATION_OPT: &str = include_str!("../fixtures/opt/minimal_evaluation.opt");

/// The template identifier [`MINIMAL_EVALUATION_OPT`] declares.
pub const MINIMAL_EVALUATION_TEMPLATE_ID: &str = "ferrobridge.minimal_evaluation.v1";

/// A synthetic canonical COMPOSITION built against
/// [`MINIMAL_EVALUATION_OPT`].
pub const MINIMAL_EVALUATION_COMPOSITION: &str =
    include_str!("../fixtures/composition/minimal_evaluation.json");

/// The `DV_TEXT` value the one element of [`MINIMAL_EVALUATION_COMPOSITION`]
/// carries.
pub const MINIMAL_EVALUATION_NOTE: &str =
    "A synthetic note committed by the FerroBRIDGE end-to-end test";

/// A synthetic FHIR R4 `Condition`.
pub const R4_CONDITION: &str = include_str!("../fixtures/fhir/r4/condition.json");

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
    fn the_composition_names_the_template_and_carries_the_note() {
        assert!(MINIMAL_EVALUATION_OPT.contains(MINIMAL_EVALUATION_TEMPLATE_ID));
        assert!(MINIMAL_EVALUATION_COMPOSITION.contains(MINIMAL_EVALUATION_TEMPLATE_ID));
        assert!(MINIMAL_EVALUATION_COMPOSITION.contains(MINIMAL_EVALUATION_NOTE));
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
