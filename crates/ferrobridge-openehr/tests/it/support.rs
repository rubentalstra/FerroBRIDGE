// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared helpers and synthetic bodies for the contract suites.
//!
//! Every body here is invented for the test: no clinical content, no real
//! identifier.

use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::commit::CommitContext;
use ferrobridge_openehr::config::{Config, RetryPolicy};
use ferrobridge_openehr::ids::template_id;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_its::rest::generated::common::UpdateAuditData;
use openehr_rm::v1_2::common::generic::party_identified::{PartyIdentified, PartyIdentifiedData};
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::composition::composition::Composition;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_rm::v1_2::data_types::text::dv_text::{DvText, DvTextData};
use std::error::Error;
use std::time::Duration;
use wiremock::MockServer;

/// A synthetic canonical COMPOSITION, the smallest one the RM admits.
pub(crate) const COMPOSITION_JSON: &str = r#"{
  "_type": "COMPOSITION",
  "name": {"_type": "DV_TEXT", "value": "Synthetic encounter"},
  "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1"},
    "template_id": {"_type": "TEMPLATE_ID", "value": "Synthetic vital signs"},
    "rm_version": "1.1.0"
  },
  "language": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_639-1"}, "code_string": "en"},
  "territory": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_3166-1"}, "code_string": "NL"},
  "category": {"_type": "DV_CODED_TEXT", "value": "event", "defining_code": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"}, "code_string": "433"}},
  "composer": {"_type": "PARTY_IDENTIFIED", "name": "Synthetic Composer"}
}"#;

/// A synthetic EHR resource.
pub(crate) const EHR_JSON: &str = r#"{
  "_type": "EHR",
  "system_id": {"_type": "HIER_OBJECT_ID", "value": "openEHRSys.example.com"},
  "ehr_id": {"_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398"},
  "ehr_status": {"_type": "OBJECT_REF", "namespace": "local", "type": "EHR_STATUS", "id": {"_type": "HIER_OBJECT_ID", "value": "3a4c2b1e-0000-4000-8000-000000000001"}},
  "ehr_access": {"_type": "OBJECT_REF", "namespace": "local", "type": "EHR_ACCESS", "id": {"_type": "HIER_OBJECT_ID", "value": "3a4c2b1e-0000-4000-8000-000000000002"}},
  "time_created": {"_type": "DV_DATE_TIME", "value": "2026-09-12T10:00:00+02:00"}
}"#;

/// A synthetic canonical OPT 1.4 operational template.
pub(crate) const OPT14_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<template xmlns="http://schemas.openehr.org/v1">
  <language>
    <terminology_id><value>ISO_639-1</value></terminology_id>
    <code_string>en</code_string>
  </language>
  <template_id><value>Synthetic vital signs</value></template_id>
  <concept>Synthetic vital signs</concept>
  <definition xsi:type="C_ARCHETYPE_ROOT" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <rm_type_name>COMPOSITION</rm_type_name>
    <occurrences>
      <lower_included>true</lower_included>
      <upper_included>true</upper_included>
      <lower_unbounded>false</lower_unbounded>
      <upper_unbounded>false</upper_unbounded>
      <lower>1</lower>
      <upper>1</upper>
    </occurrences>
    <node_id>at0000</node_id>
    <archetype_id><value>openEHR-EHR-COMPOSITION.encounter.v1</value></archetype_id>
    <template_id><value>Synthetic vital signs</value></template_id>
  </definition>
</template>"#;

/// A synthetic AOM2 operational template, the ADL 2 generation.
pub(crate) const OPT2_JSON: &str = r#"{
  "_type": "OPERATIONAL_TEMPLATE",
  "archetype_id": {
    "_type": "ARCHETYPE_HRID",
    "namespace": "org.example",
    "rm_publisher": "openEHR",
    "rm_package": "EHR",
    "rm_class": "COMPOSITION",
    "concept_id": "t_vital_signs",
    "release_version": "1.0.0",
    "version_status": "released",
    "build_count": "0"
  },
  "is_differential": false,
  "definition": {
    "_type": "C_COMPLEX_OBJECT",
    "rm_type_name": "COMPOSITION",
    "node_id": "at0000"
  },
  "terminology": {
    "_type": "ARCHETYPE_TERMINOLOGY",
    "is_differential": false,
    "original_language": "en",
    "concept_code": "at0000",
    "term_definitions": {
      "en": {
        "at0000": {"_type": "ARCHETYPE_TERM", "code": "at0000", "text": "Synthetic vital signs", "description": "A synthetic template root"}
      }
    }
  },
  "original_language": {
    "_type": "Terminology_code",
    "terminology_id": "ISO_639-1",
    "code_string": "en"
  },
  "build_uid": {"_type": "UUID", "value": "6cb19121-4307-4648-9da0-d62e4d51f19b"},
  "rm_release": "1.1.0",
  "is_generated": true,
  "other_meta_data": {}
}"#;

/// The full HRID the `adl2` route reports in its `ETag`.
pub(crate) const OPT2_HRID: &str = "org.example::openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0";

/// Returns a client for `server`, with a retry budget short enough for a test.
pub(crate) fn client(server: &MockServer) -> Result<Client, Box<dyn Error>> {
    let config = Config::new(format!("{}/v1", server.uri()).parse()?)
        .with_timeout(Duration::from_secs(5))
        .with_retry(RetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
        });
    Ok(Client::new(config)?)
}

/// Returns the synthetic composition as an RM value.
pub(crate) fn composition() -> Result<Composition, Box<dyn Error>> {
    Ok(openehr_its::json::from_canonical_json::<Composition>(
        COMPOSITION_JSON,
    )?)
}

/// Returns a code of the openEHR terminology as a `DV_CODED_TEXT`.
pub(crate) fn coded(code: &str) -> DvCodedText {
    DvCodedText {
        value: "synthetic".to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: CodePhrase {
            terminology_id: TerminologyId {
                value: "openehr".to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        },
    }
}

/// Returns an audit with a named committer, an optional demographic
/// reference and a description.
pub(crate) fn audit(change_type: &str, external_ref: Option<PartyRef>) -> UpdateAuditData {
    UpdateAuditData {
        _type: Some("UPDATE_AUDIT".to_owned()),
        system_id: None,
        change_type: coded(change_type),
        description: Some(DvText::DvText(DvTextData {
            value: "A synthetic commit".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
        })),
        committer: PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(
            PartyIdentifiedData {
                external_ref,
                name: Some("Synthetic Committer".to_owned()),
                identifiers: None,
            },
        )),
    }
}

/// Returns a commit context with every member the 1.1.0 headers carry.
pub(crate) fn commit_context() -> Result<CommitContext, Box<dyn Error>> {
    let reference = PartyRef {
        namespace: "demographic".to_owned(),
        r#type: "PERSON".to_owned(),
        id: ObjectId::HierObjectId(HierObjectId::new("bc8132ea-0000-4000-8000-000000000003")?),
    };
    Ok(CommitContext {
        lifecycle_state: Some(coded("532")),
        audit: Some(audit("251", Some(reference))),
        template_id: Some(template_id("Synthetic vital signs")?),
    })
}

/// Returns the values of request header `name` on the request `index` of
/// `server`, in wire order.
pub(crate) async fn request_header(
    server: &MockServer,
    index: usize,
    name: &str,
) -> Result<Vec<String>, Box<dyn Error>> {
    let requests = server
        .received_requests()
        .await
        .ok_or("the mock server is not recording requests")?;
    let request = requests.get(index).ok_or("no request at that index")?;
    Ok(request
        .headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_owned)
        .collect())
}
