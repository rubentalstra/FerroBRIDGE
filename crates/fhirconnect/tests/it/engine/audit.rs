// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The origin a run records in the composition's `FEEDER_AUDIT`.

use core::error::Error;

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::origin::Origin;
use fhirconnect::engine::origin::SourceItem;
use fhirconnect::engine::outcome::Warning;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::Defaults;
use fhirconnect::engine::traverse::to_openehr;

use crate::support::compiled;
use crate::support::template;

use crate::engine::condition_document;
use crate::engine::defaults;

/// Returns the defaults of a run whose origin is the synthetic sender.
fn audited(source: SourceItem) -> Defaults {
    defaults().with_origin(Origin::new("ferrobridge.test").with_source(source))
}

/// Returns the `id` of every entry of one `FEEDER_AUDIT` identifier list.
fn audit_ids<'value>(audit: &'value serde_json::Value, list: &str) -> Vec<&'value str> {
    audit
        .get(list)
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_run_with_an_origin_records_it_in_the_feeder_audit() -> Result<(), Box<dyn Error>> {
    // RM 1.1.0 common.html §FEEDER_AUDIT: the audit "describes the origin of
    // data that have been transformed into openEHR form", and
    // FEEDER_AUDIT_DETAILS.system_id names the system that handled it.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &audited(
            SourceItem::new("Condition")
                .with_id("sender-1")
                .with_version_id("3"),
        ),
        &CallContext::new(),
    )?;
    let audit = inbound
        .value()
        .value()
        .get("feeder_audit")
        .ok_or("the composition carries a FEEDER_AUDIT")?;
    assert_eq!(
        audit
            .pointer("/originating_system_audit/system_id")
            .and_then(serde_json::Value::as_str),
        Some("ferrobridge.test")
    );
    assert_eq!(
        audit
            .pointer("/originating_system_audit/version_id")
            .and_then(serde_json::Value::as_str),
        Some("3"),
        "the source meta.versionId travels as the originating version"
    );
    assert_eq!(
        audit_ids(audit, "originating_system_item_ids"),
        ["sender-1"],
        "the source resource id travels as the one originating item: {audit}"
    );
    assert_eq!(
        audit
            .pointer("/originating_system_item_ids/0/type")
            .and_then(serde_json::Value::as_str),
        Some("Condition")
    );
    assert_eq!(
        audit
            .pointer("/feeder_system_audit/system_id")
            .and_then(serde_json::Value::as_str),
        Some("ferrobridge.test")
    );
    Ok(())
}

#[test]
fn the_defaulted_warnings_and_the_feeder_audit_agree() -> Result<(), Box<dyn Error>> {
    // defaults-for-fields.adoc names the fields the engine fills; each one is
    // a declared Warning::Defaulted and one feeder_system_item_ids entry, in
    // the same order.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &audited(SourceItem::new("Condition").with_id("sender-1")),
        &CallContext::new(),
    )?;
    let warned: Vec<&str> = inbound
        .warnings()
        .iter()
        .filter_map(|warning| match *warning {
            Warning::Defaulted { ref field } => Some(field.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !warned.is_empty(),
        "the minimal chain leaves fields to default"
    );
    let audit = inbound
        .value()
        .value()
        .get("feeder_audit")
        .ok_or("the composition carries a FEEDER_AUDIT")?;
    assert_eq!(
        audit_ids(audit, "feeder_system_item_ids"),
        warned,
        "the audit lists exactly the defaulted fields, in order: {audit}"
    );
    let types: Vec<Option<&str>> = audit
        .get("feeder_system_item_ids")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| entry.get("type").and_then(serde_json::Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        types.iter().all(|kind| *kind == Some("defaulted")),
        "every feeder item is typed as a default: {types:?}"
    );
    // The builder validated the composition against its template before it
    // answered; reading it back through the same validation shows the audit
    // is admitted content.
    index.accept(inbound.value().value().clone())?;
    Ok(())
}

#[test]
fn a_source_with_no_id_is_recorded_as_unknown() -> Result<(), Box<dyn Error>> {
    // An absent identifier is recorded, never invented.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &audited(SourceItem::new("Condition")),
        &CallContext::new(),
    )?;
    let audit = inbound
        .value()
        .value()
        .get("feeder_audit")
        .ok_or("the composition carries a FEEDER_AUDIT")?;
    assert_eq!(audit_ids(audit, "originating_system_item_ids"), ["unknown"]);
    assert_eq!(
        audit.pointer("/originating_system_audit/version_id"),
        None,
        "a source with no version records none"
    );
    Ok(())
}

#[test]
fn a_run_without_an_origin_writes_no_feeder_audit() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(inbound.value().value().get("feeder_audit"), None);
    Ok(())
}
