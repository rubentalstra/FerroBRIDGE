// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The `CapabilityStatement` the facade serves at `GET [base]/metadata`.
//!
//! It names exactly what the loaded programs support
//! (<https://hl7.org/fhir/R4/capabilitystatement.html>): one `rest.resource`
//! per resource type a program maps, with the interactions this milestone
//! implements, the `$validate` operation, and no search parameter. A type with
//! no program is absent here and answers `404` with `not-supported` on the
//! wire (`docs/architecture.md` §4.6).
//!
//! The statement declares `updateCreate: false`, because a `PUT` to an
//! unknown id is a `404` rather than a create, and `conditionalCreate` and
//! `conditionalUpdate` true, because `If-None-Exist` and `If-Match` are
//! honoured as R4 defines them (<https://hl7.org/fhir/R4/http.html>).

use fhir_types::r4::capability_statement::CapabilityStatement;
use fhir_types::r4::capability_statement::CapabilityStatementRest;
use fhir_types::r4::capability_statement::CapabilityStatementRestInteraction;
use fhir_types::r4::capability_statement::CapabilityStatementRestResource;
use fhir_types::r4::capability_statement::CapabilityStatementRestResourceInteraction;
use fhir_types::r4::capability_statement::CapabilityStatementRestResourceOperation;
use fhir_types::r4::capability_statement::CapabilityStatementSoftware;

use crate::facade::programs::Programs;
use crate::state;

/// The FHIR release this facade speaks.
///
/// The value is a code of `http://hl7.org/fhir/ValueSet/FHIR-version`, and R4
/// is `4.0.1` (<https://hl7.org/fhir/R4/capabilitystatement.html>).
pub const FHIR_VERSION: &str = "4.0.1";

/// The date this capability description was published.
///
/// The statement is compiled in and changes with the build, so two calls to
/// `GET [base]/metadata` are byte-identical.
const PUBLISHED: &str = "2026-01-01T00:00:00Z";

/// The canonical `OperationDefinition` of `$validate`.
const VALIDATE_DEFINITION: &str = "http://hl7.org/fhir/OperationDefinition/Resource-validate";

/// The type-level interactions this facade implements.
///
/// The codes are of `http://hl7.org/fhir/ValueSet/type-restful-interaction`
/// (<https://hl7.org/fhir/R4/capabilitystatement.html>).
const RESOURCE_INTERACTIONS: [&str; 3] = ["create", "read", "update"];

/// Returns the statement the loaded `programs` support.
///
/// `base` is the absolute URL the facade is reachable at, which the statement
/// records as its implementation description.
#[must_use]
pub fn statement(programs: &Programs, base: &str) -> CapabilityStatement {
    CapabilityStatement {
        status: "active".into(),
        // NOTE: `date` is "the date when the capability statement was
        // published" (<https://hl7.org/fhir/R4/capabilitystatement.html>), and
        // this one is published by the build, so it is not a clock reading.
        date: PUBLISHED.into(),
        kind: "instance".into(),
        fhir_version: FHIR_VERSION.into(),
        format: vec![
            crate::facade::media::FHIR_JSON.into(),
            crate::facade::media::JSON.into(),
        ],
        software: Some(CapabilityStatementSoftware {
            name: state::PRODUCT.into(),
            version: Some(state::VERSION.into()),
            ..CapabilityStatementSoftware::default()
        }),
        implementation: Some(
            fhir_types::r4::capability_statement::CapabilityStatementImplementation {
                description: "The FerroBRIDGE FHIR facade over an openEHR CDR".into(),
                url: Some(base.into()),
                ..fhir_types::r4::capability_statement::CapabilityStatementImplementation::default()
            },
        ),
        rest: vec![CapabilityStatementRest {
            mode: "server".into(),
            resource: programs
                .resource_types()
                .iter()
                .map(|name| resource(name))
                .collect(),
            // NOTE: `transaction` is a system-level interaction of
            // `http://hl7.org/fhir/ValueSet/system-restful-interaction`; batch
            // is absent because this milestone does not implement it.
            interaction: vec![CapabilityStatementRestInteraction {
                code: "transaction".into(),
                ..CapabilityStatementRestInteraction::default()
            }],
            ..CapabilityStatementRest::default()
        }],
        ..CapabilityStatement::default()
    }
}

/// Returns the `rest.resource` entry of one supported type.
fn resource(name: &str) -> CapabilityStatementRestResource {
    CapabilityStatementRestResource {
        r#type: name.into(),
        interaction: RESOURCE_INTERACTIONS
            .iter()
            .map(|code| CapabilityStatementRestResourceInteraction {
                code: (*code).into(),
                ..CapabilityStatementRestResourceInteraction::default()
            })
            .collect(),
        update_create: Some(false.into()),
        conditional_create: Some(true.into()),
        conditional_update: Some(true.into()),
        operation: vec![CapabilityStatementRestResourceOperation {
            name: "validate".into(),
            definition: VALIDATE_DEFINITION.into(),
            ..CapabilityStatementRestResourceOperation::default()
        }],
        ..CapabilityStatementRestResource::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{FHIR_VERSION, statement};
    use crate::facade::programs::Programs;

    #[test]
    fn an_empty_registry_names_no_resource_type() {
        let built = statement(&Programs::default(), "http://localhost:8080/fhir");
        let rest = built.rest.first().expect("one rest entry");
        assert!(
            rest.resource.is_empty(),
            "a facade with no program names a resource type"
        );
        assert_eq!(Some("server"), rest.mode.value.as_deref());
    }

    #[test]
    fn the_statement_pins_the_release_and_the_media_types() {
        let built = statement(&Programs::default(), "http://localhost:8080/fhir");
        assert_eq!(Some(FHIR_VERSION), built.fhir_version.value.as_deref());
        let formats: Vec<&str> = built
            .format
            .iter()
            .filter_map(|code| code.value.as_deref())
            .collect();
        assert!(formats.contains(&"application/fhir+json"), "{formats:?}");
    }

    #[test]
    fn the_system_level_interaction_is_transaction_and_never_batch() {
        let built = statement(&Programs::default(), "http://localhost:8080/fhir");
        let codes: Vec<&str> = built
            .rest
            .first()
            .expect("one rest entry")
            .interaction
            .iter()
            .filter_map(|entry| entry.code.value.as_deref())
            .collect();
        assert_eq!(vec!["transaction"], codes);
    }

    #[test]
    fn two_readings_of_the_statement_are_identical() {
        let first = statement(&Programs::default(), "http://localhost:8080/fhir");
        let second = statement(&Programs::default(), "http://localhost:8080/fhir");
        assert_eq!(first, second);
    }
}
