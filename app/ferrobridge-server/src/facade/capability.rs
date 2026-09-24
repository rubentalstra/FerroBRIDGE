// SPDX-FileCopyrightText: Vernum Projecten B.V.
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

/// The canonical `OperationDefinition` of `$tofhir`.
///
/// `ToFhir.fsh` of the draft REST API chapter (FHIRconnect pull request #93)
/// assigns no `url`, so this is the IG convention of canonical base plus
/// `/<ResourceType>/<id>` (<https://hl7.org/fhir/R4/references.html#canonical>,
/// <https://fshschool.org/docs/sushi/configuration/>) over the instance id
/// `ToFhir` and the `canonical: http://fhirconnect.org/fhir` of
/// `docs/specs/fhirconnect/draft-rest-api/rest/sushi-config.yaml`.
pub const TOFHIR_DEFINITION: &str = "http://fhirconnect.org/fhir/OperationDefinition/ToFhir";

/// The canonical `OperationDefinition` of `$toopenehr`.
///
/// Derived as [`TOFHIR_DEFINITION`] is, from the instance id `ToOpenEhr` of
/// `ToOpenEhr.fsh`.
pub const TOOPENEHR_DEFINITION: &str = "http://fhirconnect.org/fhir/OperationDefinition/ToOpenEhr";

/// Whether the FHIRconnect operations lane answers beside the facade.
///
/// The facade and the operations lane switch independently, so the router
/// tells the statement which of the two it serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operations {
    /// `$tofhir` and `$toopenehr` are served under the same base.
    Served,
    /// No operations lane is configured.
    Absent,
}

/// Returns the statement the loaded `programs` support.
///
/// `base` is the absolute URL the facade is reachable at, which the statement
/// records as its implementation description. With [`Operations::Served`],
/// `rest.operation` names `$tofhir` and `$toopenehr`, the two operations the
/// draft chapter defines with `system = true`
/// (<https://hl7.org/fhir/R4/capabilitystatement.html>,
/// `CapabilityStatement.rest.operation`). Their direct forms are never named:
/// the chapter calls them "a deliberate deviation from the FHIR Operations
/// framework" and keeps them out of its implementation guide.
#[must_use]
pub fn statement(programs: &Programs, base: &str, operations: Operations) -> CapabilityStatement {
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
            operation: system_operations(operations),
            ..CapabilityStatementRest::default()
        }],
        ..CapabilityStatement::default()
    }
}

/// Returns the system-level `rest.operation` entries `operations` serves.
fn system_operations(operations: Operations) -> Vec<CapabilityStatementRestResourceOperation> {
    match operations {
        Operations::Absent => Vec::new(),
        Operations::Served => [
            ("tofhir", TOFHIR_DEFINITION),
            ("toopenehr", TOOPENEHR_DEFINITION),
        ]
        .into_iter()
        .map(
            |(name, definition)| CapabilityStatementRestResourceOperation {
                name: name.into(),
                definition: definition.into(),
                ..CapabilityStatementRestResourceOperation::default()
            },
        )
        .collect(),
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
    use super::{FHIR_VERSION, Operations, TOFHIR_DEFINITION, TOOPENEHR_DEFINITION, statement};
    use crate::facade::programs::Programs;

    #[test]
    fn an_empty_registry_names_no_resource_type() {
        let built = statement(
            &Programs::default(),
            "http://localhost:8080/fhir",
            Operations::Absent,
        );
        let rest = built.rest.first().expect("one rest entry");
        assert!(
            rest.resource.is_empty(),
            "a facade with no program names a resource type"
        );
        assert_eq!(Some("server"), rest.mode.value.as_deref());
    }

    #[test]
    fn the_statement_pins_the_release_and_the_media_types() {
        let built = statement(
            &Programs::default(),
            "http://localhost:8080/fhir",
            Operations::Absent,
        );
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
        let built = statement(
            &Programs::default(),
            "http://localhost:8080/fhir",
            Operations::Absent,
        );
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
        let first = statement(
            &Programs::default(),
            "http://localhost:8080/fhir",
            Operations::Absent,
        );
        let second = statement(
            &Programs::default(),
            "http://localhost:8080/fhir",
            Operations::Absent,
        );
        assert_eq!(first, second);
    }

    /// Returns the names and definitions of the system-level operations.
    fn system_operations(operations: Operations) -> Vec<(String, String)> {
        statement(
            &Programs::default(),
            "http://localhost:8080/fhir",
            operations,
        )
        .rest
        .first()
        .expect("one rest entry")
        .operation
        .iter()
        .map(|entry| {
            (
                entry.name.value.clone().unwrap_or_default(),
                entry.definition.value.clone().unwrap_or_default(),
            )
        })
        .collect()
    }

    #[test]
    fn a_served_operations_lane_declares_both_system_level_operations() {
        assert_eq!(
            vec![
                (String::from("tofhir"), String::from(TOFHIR_DEFINITION)),
                (
                    String::from("toopenehr"),
                    String::from(TOOPENEHR_DEFINITION)
                ),
            ],
            system_operations(Operations::Served)
        );
    }

    #[test]
    fn an_absent_operations_lane_declares_no_system_level_operation() {
        assert!(system_operations(Operations::Absent).is_empty());
    }
}
