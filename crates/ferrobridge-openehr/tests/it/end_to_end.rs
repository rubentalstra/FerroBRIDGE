// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The client against a real openEHR CDR in a container.
//!
//! One EHR, one template, one composition and one query travel the whole
//! ITS-REST 1.1.0 surface the client implements, so the contract suites beside
//! this file are joined by a run against a server that answers for itself. The
//! test runs only when `FERROBRIDGE_E2E=1` admits the container harness.

use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::commit::{ChangeType, CommitContext, Committer, LifecycleState};
use ferrobridge_openehr::composition::{
    CompositionOutcome, CreateCompositionOutcome, DeleteCompositionOutcome, UidBasedId,
    UpdateCompositionOutcome,
};
use ferrobridge_openehr::config::Config;
use ferrobridge_openehr::ehr::{CreateEhrOutcome, EhrOutcome};
use ferrobridge_openehr::ids::{EhrId, ObjectVersionId, SubjectId, SubjectNamespace, TemplateId};
use ferrobridge_openehr::prefer::{Prefer, Returned};
use ferrobridge_openehr::query::QueryOutcome;
use ferrobridge_openehr::template::{TemplateOutcome, TemplateSource};
use ferrobridge_testkit::containers;
use ferrobridge_testkit::fixtures;
use openehr_its::rest::generated::query::AdhocQueryExecute;
use openehr_rm::v1_2::composition::composition::Composition;
use openehr_rm::v1_2::ehr::ehr_status::EhrStatus;
use std::error::Error;

/// The namespace of the synthetic subject.
const SUBJECT_NAMESPACE: &str = "ferrobridge-test";

/// The synthetic subject the EHR is created for.
const SUBJECT_ID: &str = "synthetic-subject-0001";

/// The query that finds every composition of the repository.
const AQL: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

/// `creation` in the openEHR Audit Change Type vocabulary.
const CREATION: &str = "249";

/// `modification` in the openEHR Audit Change Type vocabulary.
const MODIFICATION: &str = "251";

/// `complete` in the openEHR Version Lifecycle State vocabulary.
const COMPLETE: &str = "532";

/// A synthetic `EHR_STATUS` naming the subject by an external reference.
const EHR_STATUS_JSON: &str = r#"{
  "_type": "EHR_STATUS",
  "name": {"_type": "DV_TEXT", "value": "EHR Status"},
  "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1"},
    "rm_version": "1.1.0"
  },
  "subject": {
    "_type": "PARTY_SELF",
    "external_ref": {
      "_type": "PARTY_REF",
      "namespace": "ferrobridge-test",
      "type": "PERSON",
      "id": {"_type": "GENERIC_ID", "value": "synthetic-subject-0001", "scheme": "ferrobridge-synthetic"}
    }
  },
  "is_queryable": true,
  "is_modifiable": true
}"#;

#[tokio::test]
async fn the_client_commits_reads_updates_queries_and_deletes_against_a_real_cdr()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let cdr = containers::cdr().await?;
    let client = Client::new(Config::new(cdr.base_url().parse()?))?;
    let template_id = TemplateId::new(fixtures::MINIMAL_EVALUATION_TEMPLATE_ID)?;

    // The fixture parses locally before the CDR is asked to accept it, so a
    // refusal is attributable to the server rather than to the fixture.
    let parsed = openehr_its::opt14::from_xml(fixtures::MINIMAL_EVALUATION_OPT)?;
    assert_eq!(
        fixtures::MINIMAL_EVALUATION_TEMPLATE_ID,
        parsed.template_id.value,
        "the fixture does not declare the template id the test uses"
    );
    upload_template(cdr.base_url()).await?;

    let ehr_id = create_ehr(&client).await?;
    find_by_subject(&client, &ehr_id).await?;
    read_template(&client, &template_id).await?;

    let version_id = commit(&client, &ehr_id, &template_id).await?;
    read_back(&client, &ehr_id, &version_id).await?;
    let second = update(&client, &ehr_id, &version_id, &template_id).await?;
    refuse_a_stale_if_match(&client, &ehr_id, &version_id, &template_id).await?;
    query_finds_the_composition(&client, &second).await?;
    delete_and_read_the_deleted_outcome(&client, &ehr_id, &second).await?;
    Ok(())
}

/// Uploads the synthetic operational template through the CDR's own route.
///
/// The client has no template upload (`ferrobridge-openehr` reads templates
/// and never writes them), so this is a plain `POST` of the canonical OPT 1.4
/// XML to `/definition/template/adl1.4`
/// (<https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/definition.html>).
async fn upload_template(base_url: &str) -> Result<(), Box<dyn Error>> {
    let response = reqwest::Client::new()
        .post(format!("{base_url}/definition/template/adl1.4"))
        .header(reqwest::header::CONTENT_TYPE, "application/xml")
        .body(fixtures::MINIMAL_EVALUATION_OPT)
        .send()
        .await?;
    assert_eq!(
        reqwest::StatusCode::CREATED,
        response.status(),
        "the CDR refused the synthetic template: {}",
        response.text().await?
    );
    Ok(())
}

/// Creates the EHR of the synthetic subject and returns its identifier.
async fn create_ehr(client: &Client) -> Result<EhrId, Box<dyn Error>> {
    let status = openehr_its::json::from_canonical_json::<EhrStatus>(EHR_STATUS_JSON)?;
    let outcome = client
        .create_ehr(Some(&status), Prefer::Representation)
        .await?;
    match outcome {
        CreateEhrOutcome::Created { ehr_id, returned } => {
            assert!(
                matches!(returned, Returned::Representation(_)),
                "the representation was asked for and not returned"
            );
            Ok(ehr_id)
        }
        other => Err(format!("expected a created EHR, got {other:?}").into()),
    }
}

/// Finds the same EHR through its subject.
async fn find_by_subject(client: &Client, ehr_id: &EhrId) -> Result<(), Box<dyn Error>> {
    let outcome = client
        .ehr_by_subject(
            &SubjectId::new(SUBJECT_ID)?,
            &SubjectNamespace::new(SUBJECT_NAMESPACE)?,
        )
        .await?;
    match outcome {
        EhrOutcome::Found(ehr) => {
            assert_eq!(
                ehr_id.as_str(),
                ehr.ehr_id.value(),
                "the subject found a different EHR"
            );
            Ok(())
        }
        other => Err(format!("the subject did not find its EHR: {other:?}").into()),
    }
}

/// Reads the uploaded template back and builds its Web Template.
async fn read_template(client: &Client, template_id: &TemplateId) -> Result<(), Box<dyn Error>> {
    let outcome = client.template(template_id).await?;
    let TemplateOutcome::Found(TemplateSource::Opt14(template)) = outcome else {
        return Err(format!("expected the adl1.4 route to answer, got {outcome:?}").into());
    };
    assert_eq!(
        fixtures::MINIMAL_EVALUATION_TEMPLATE_ID,
        template.template_id.value,
        "the CDR served a different template"
    );
    let web_template = openehr_its::flat::webtemplate::builder::build_web_template(&template)?;
    assert_eq!(
        fixtures::MINIMAL_EVALUATION_TEMPLATE_ID,
        web_template.template_id,
        "the Web Template names a different template"
    );
    Ok(())
}

/// Commits the synthetic composition and returns the version it became.
async fn commit(
    client: &Client,
    ehr_id: &EhrId,
    template_id: &TemplateId,
) -> Result<ObjectVersionId, Box<dyn Error>> {
    let outcome = client
        .create_composition(
            ehr_id,
            &composition()?,
            &commit_context(CREATION, template_id)?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        CreateCompositionOutcome::Created {
            version_id,
            returned,
        } => {
            let Returned::Representation(committed) = returned else {
                return Err("the representation was asked for and not returned".into());
            };
            assert_eq!(
                Some(version_id.to_string()),
                committed.uid.as_ref().map(|uid| uid.value().to_owned()),
                "the committed composition does not carry the version the ETag named"
            );
            Ok(version_id)
        }
        other => Err(format!("expected a created composition, got {other:?}").into()),
    }
}

/// Reads the committed composition back and compares it with what was sent.
///
/// The server assigns `uid`, which the request has none of, so the comparison
/// is of the composition with that one server-assigned member removed.
async fn read_back(
    client: &Client,
    ehr_id: &EhrId,
    version_id: &ObjectVersionId,
) -> Result<(), Box<dyn Error>> {
    let outcome = client
        .composition(ehr_id, &UidBasedId::Version(version_id.clone()), None)
        .await?;
    match outcome {
        CompositionOutcome::Found {
            version_id: read_version,
            composition: read,
        } => {
            assert_eq!(
                Some(version_id.clone()),
                read_version,
                "the read answered with a different version in its ETag"
            );
            let mut read = *read;
            assert_eq!(
                Some(version_id.to_string()),
                read.uid.as_ref().map(|uid| uid.value().to_owned()),
                "the composition read back does not carry the version it was read at"
            );
            read.uid = None;
            assert_eq!(
                composition()?,
                read,
                "the composition read back is not the one committed"
            );
            Ok(())
        }
        other => Err(format!("expected the composition, got {other:?}").into()),
    }
}

/// Commits a second version under the `ETag` of the first.
async fn update(
    client: &Client,
    ehr_id: &EhrId,
    preceding: &ObjectVersionId,
    template_id: &TemplateId,
) -> Result<ObjectVersionId, Box<dyn Error>> {
    let outcome = client
        .update_composition(
            ehr_id,
            &preceding.versioned_object_uid(),
            preceding,
            &composition()?,
            &commit_context(MODIFICATION, template_id)?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        UpdateCompositionOutcome::Updated { version_id, .. } => {
            assert_eq!(
                "2",
                version_id.version_tree_id(),
                "the update did not become the second version"
            );
            Ok(version_id)
        }
        other => Err(format!("expected an updated composition, got {other:?}").into()),
    }
}

/// Commits under the version that is no longer the latest.
async fn refuse_a_stale_if_match(
    client: &Client,
    ehr_id: &EhrId,
    stale: &ObjectVersionId,
    template_id: &TemplateId,
) -> Result<(), Box<dyn Error>> {
    let outcome = client
        .update_composition(
            ehr_id,
            &stale.versioned_object_uid(),
            stale,
            &composition()?,
            &commit_context(MODIFICATION, template_id)?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        UpdateCompositionOutcome::PreconditionFailed {
            latest_version_id, ..
        } => {
            assert_eq!(
                Some("2"),
                latest_version_id
                    .as_ref()
                    .map(ObjectVersionId::version_tree_id),
                "the 412 did not name the latest version in its ETag"
            );
            Ok(())
        }
        other => Err(format!("expected a precondition failure, got {other:?}").into()),
    }
}

/// Finds the committed composition through AQL.
async fn query_finds_the_composition(
    client: &Client,
    version_id: &ObjectVersionId,
) -> Result<(), Box<dyn Error>> {
    let request = AdhocQueryExecute {
        q: AQL.to_owned(),
        offset: None,
        fetch: None,
        query_parameters: None,
    };
    let outcome = client.query_aql(&request).await?;
    match outcome {
        QueryOutcome::Rows(set) => {
            let wanted = serde_json::Value::String(version_id.to_string());
            assert!(
                set.rows.iter().any(|row| row.contains(&wanted)),
                "the query did not return the committed composition: {:?}",
                set.rows
            );
            Ok(())
        }
        other => Err(format!("expected a result set, got {other:?}").into()),
    }
}

/// Deletes the composition and reads the deleted outcome back.
///
/// "A `GET` … with `version_at_time` … returns `204 No Content` if the
/// composition has been deleted" (ITS-REST 1.1.0, `composition_get`), which
/// the client answers as [`CompositionOutcome::Deleted`] rather than as an
/// absent value.
async fn delete_and_read_the_deleted_outcome(
    client: &Client,
    ehr_id: &EhrId,
    latest: &ObjectVersionId,
) -> Result<(), Box<dyn Error>> {
    let outcome = client.delete_composition(ehr_id, latest).await?;
    match outcome {
        DeleteCompositionOutcome::Deleted { version_id } => {
            assert_eq!(
                Some("3"),
                version_id.as_ref().map(ObjectVersionId::version_tree_id),
                "the delete did not report the version it created"
            );
        }
        other => return Err(format!("expected a deleted composition, got {other:?}").into()),
    }
    let outcome = client
        .composition(
            ehr_id,
            &UidBasedId::VersionedObject(latest.versioned_object_uid()),
            None,
        )
        .await?;
    match outcome {
        CompositionOutcome::Deleted => Ok(()),
        other => Err(format!("expected the deleted outcome, got {other:?}").into()),
    }
}

/// Returns the commit metadata of one commit, with `change_type`.
///
/// The audit change type travels in `openehr-audit-details` and the lifecycle
/// state in `openehr-version` (ITS-REST 1.1.0 §Requests and responses/HTTP
/// headers/openehr-version and openehr-audit-details).
fn commit_context(
    change_type: &str,
    template_id: &TemplateId,
) -> Result<CommitContext, Box<dyn Error>> {
    Ok(CommitContext {
        lifecycle_state: Some(LifecycleState::new(COMPLETE)?),
        change_type: Some(ChangeType::new(change_type)?),
        committer: Some(Committer {
            name: "Synthetic Committer".to_owned(),
            external_ref: None,
        }),
        description: Some("A synthetic commit".to_owned()),
        system_id: None,
        template_id: Some(template_id.clone()),
    })
}

/// Returns the synthetic composition as an RM value.
fn composition() -> Result<Composition, Box<dyn Error>> {
    Ok(openehr_its::json::from_canonical_json::<Composition>(
        fixtures::MINIMAL_EVALUATION_COMPOSITION,
    )?)
}
