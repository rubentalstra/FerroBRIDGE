// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Turning a stored composition into the resource the facade answers with.
//!
//! The engine maps the composition back to FHIR, and this module adds the
//! facts the engine has no way to know: the logical id the identity map
//! holds, the `meta.versionId` the CDR's version tree names, the
//! `meta.source` that points at the composition version the content came
//! from, and the subject the identity map binds to the composition's EHR when
//! the mapping wrote none (no specification governs the identity: our own
//! design). The answer is then checked against the required elements of the
//! element table, so no interaction answers an invalid instance.

use crate::cdr::ids::EhrId;
use fhir_types::codec::Object;
use fhir_types::codec::Value;
use fhirconnect::engine::outcome::Warning;
use fhirconnect::resolve::program::Program;
use http::StatusCode;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;
use url::Url;

use crate::facade::Facade;
use crate::facade::engine;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::complete;
use crate::facade::identity::FhirResourceId;
use crate::facade::ingest::Refused;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::reply;
use crate::facade::status;

/// What one rendered resource carries beside its body.
#[derive(Debug, Clone)]
pub(crate) struct Rendered {
    /// The resource, ready for the wire.
    pub(crate) body: Object,
    /// The losses the run declared, as issues a caller may report.
    pub(crate) warnings: Vec<Warning>,
    /// The element paths the facade filled from the identity binding.
    pub(crate) filled: Vec<&'static str>,
}

/// Returns the subject the identity map binds to `ehr_id`, as the reference
/// a rendered resource names it by, when the map knows one.
///
/// # Errors
///
/// The refusal an identity-store failure renders as.
pub(crate) fn subject_of(facade: &Facade, ehr_id: &EhrId) -> Result<Option<Value>, Refusal> {
    let person = facade
        .store()
        .person_of(ehr_id)
        .map_err(|error| crate::facade::handlers::write::store_refusal(&error))?;
    Ok(person.map(|person| complete::reference_to(&person, &facade.settings().subject_namespace)))
}

/// Where one rendered resource comes from: the composition version, the
/// logical id the map holds, and the subject the map binds to its EHR.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Origin<'a> {
    /// The logical id the identity map holds.
    pub(crate) id: &'a FhirResourceId,
    /// The composition version the content came from.
    pub(crate) version: &'a ObjectVersionId,
    /// The ITS-REST URL of that version.
    pub(crate) source: &'a Url,
    /// The subject reference [`subject_of`] answered, when the map knows one.
    pub(crate) subject: Option<&'a Value>,
}

/// Returns the resource `composition` maps to, decorated with its identity
/// and checked against the required elements R4 gives its type.
///
/// # Errors
///
/// The engine's refusal, and the `500` of [`complete::refusal`] naming every
/// required element neither the mapping nor the binding supplies.
pub(crate) fn render(
    program: &Program,
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    origin: Origin<'_>,
) -> Result<Rendered, Refusal> {
    let Origin {
        id,
        version,
        source,
        subject,
    } = origin;
    let resource_type = program.resource().as_str();
    let produced = engine::outbound(program, index, composition)
        .map_err(|error| Refusal::from(crate::facade::ingest::engine_refusal(&error)))?;
    let warnings = produced.warnings().to_vec();
    let (value, created) = produced.into_parts();
    let mut body = resource_object(value, resource_type)?;
    contain(&mut body, created);
    let filled: Vec<&'static str> = complete::fill_subject(&mut body, resource_type, subject)
        .into_iter()
        .collect();
    body.insert(String::from("id"), Value::String(String::from(id.as_str())));
    let mut meta = body
        .get("meta")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    meta.insert(
        String::from("versionId"),
        Value::String(String::from(version.version_tree_id().value())),
    );
    // NOTE: `Meta.source` is "a uri that identifies the source system"
    // (<https://hl7.org/fhir/R4/resource.html#Meta>), so the openEHR version
    // uid travels as the ITS-REST URL of that exact version.
    meta.insert(String::from("source"), Value::String(source.to_string()));
    meta.insert(
        String::from("profile"),
        Value::Array(vec![Value::String(String::from(
            program.profile().url().as_str(),
        ))]),
    );
    body.insert(String::from("meta"), Value::Object(meta));
    let absent = complete::absent_required(&body, resource_type);
    if !absent.is_empty() {
        tracing::error!(
            resource_type,
            absent = absent.join(", "),
            "the rendered resource lacks elements R4 requires"
        );
        return Err(Refusal::from(complete::refusal(resource_type, &absent)));
    }
    Ok(Rendered {
        body,
        warnings,
        filled,
    })
}

/// Returns the resource object an outbound run produced.
///
/// The value is mapped clinical content, so a refusal names only its shape:
/// the resource type the program maps and the JSON kind the engine produced.
/// The status is the table's [`status::INTERNAL`] row, because the engine is
/// the bridge's own.
fn resource_object(value: Value, resource_type: &str) -> Result<Object, Refusal> {
    let kind = match value {
        Value::Object(object) => return Ok(object),
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
    };
    Err(Refusal::from(Refused::of_answer(&status::Answer::new(
        status::INTERNAL,
        format!("the engine produced {kind} where a {resource_type} resource object belongs"),
    ))))
}

/// Carries the resources an outbound run created inside the one it mapped.
///
/// The facade serves one resource per composition entry, and a resource the
/// run created beside it has no location of its own on this server, so it
/// travels as a contained resource and every reference to it becomes the
/// local `#<id>` form (<https://hl7.org/fhir/R4/references.html#contained>).
/// No specification governs the choice: our own design.
fn contain(body: &mut Object, created: Vec<Value>) {
    if created.is_empty() {
        return;
    }
    let mut local: Vec<(String, String)> = Vec::with_capacity(created.len());
    for resource in &created {
        if let (Some(kind), Some(id)) = (
            resource.get("resourceType").and_then(Value::as_str),
            resource.get("id").and_then(Value::as_str),
        ) {
            local.push((format!("{kind}/{id}"), format!("#{id}")));
        }
    }
    let mut contained: Vec<Value> = body
        .get("contained")
        .and_then(Value::as_array)
        .map(<[Value]>::to_vec)
        .unwrap_or_default();
    contained.extend(created);
    for resource in &mut contained {
        relocate(resource, &local);
    }
    let mut document = Value::Object(core::mem::take(body));
    relocate(&mut document, &local);
    if let Value::Object(mut object) = document {
        object.insert(String::from("contained"), Value::Array(contained));
        *body = object;
    }
}

/// Rewrites every `reference` that names a contained resource to its local
/// form.
fn relocate(value: &mut Value, local: &[(String, String)]) {
    match *value {
        Value::Object(ref mut object) => {
            for (key, member) in object.iter_mut() {
                if key == "reference"
                    && let Value::String(ref mut text) = *member
                    && let Some((_, rewritten)) = local.iter().find(|(from, _)| from == text)
                {
                    *text = rewritten.clone();
                    continue;
                }
                if key != "contained" {
                    relocate(member, local);
                }
            }
        }
        Value::Array(ref mut items) => {
            for item in items {
                relocate(item, local);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Records what the facade filled on one rendered resource and the declared
/// set of losses its outbound run took.
///
/// A fill is counted with the element paths it wrote, and a warning is a loss
/// the specification itself declares, never a swallowed failure, carrying a
/// mapping name and an element path rather than a value, so neither line
/// holds clinical content.
pub(crate) fn log_outcome(rendered: &Rendered) {
    if !rendered.filled.is_empty() {
        tracing::info!(
            filled = rendered.filled.len(),
            elements = rendered.filled.join(", "),
            "the facade filled required elements from the identity binding"
        );
    }
    if rendered.warnings.is_empty() {
        return;
    }
    tracing::debug!(
        declared = rendered.warnings.len(),
        losses = rendered
            .warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>()
            .join("; "),
        "the outbound mapping declared losses"
    );
}

/// Returns the ITS-REST URL of one composition version.
pub(crate) fn composition_url(
    base: &Url,
    ehr_id: &EhrId,
    version: &ObjectVersionId,
) -> Result<Url, Refusal> {
    base.join(&format!(
        "ehr/{}/composition/{}",
        ehr_id.as_str(),
        version.value()
    ))
    .map_err(|error| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the composition URL could not be built from the configured CDR base: {error}"
            )),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{composition_url, contain, resource_object};
    use crate::cdr::ids::EhrId;
    use fhir_types::codec::Value;
    use http::StatusCode;
    use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;

    /// A synthetic value no refusal may carry onto the wire.
    const MARKER: &str = "ferrobridge-synthetic-clinical-marker";

    #[tokio::test]
    async fn an_output_that_is_no_object_is_refused_by_its_shape_alone() {
        let produced = Value::Array(vec![Value::String(String::from(MARKER))]);
        let refusal =
            resource_object(produced, "Condition").expect_err("an array is no resource object");
        let response = refusal.into_response();
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, response.status());
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("the refusal body reads");
        let text = String::from_utf8(bytes.to_vec()).expect("the body is UTF-8");
        assert!(
            text.contains("the engine produced an array where a Condition resource object belongs"),
            "{text}"
        );
        assert!(
            !text.contains(MARKER),
            "the produced value reached the wire: {text}"
        );
    }

    #[test]
    fn an_object_output_passes_through() {
        let Value::Object(object) = Value::from_serde_json(serde_json::json!({
            "resourceType": "Condition"
        })) else {
            panic!("the literal is an object");
        };
        let passed = resource_object(Value::Object(object.clone()), "Condition")
            .expect("an object is the resource");
        assert_eq!(object, passed);
    }

    #[test]
    fn a_created_resource_is_contained_and_referenced_locally() {
        // R4 references.html#contained: a contained resource is referenced
        // as `#<id>` from the resource that contains it.
        let Value::Object(mut body) = Value::from_serde_json(serde_json::json!({
            "resourceType": "Condition",
            "evidence": [{"detail": [{"reference": "Observation/created-1"}]}]
        })) else {
            panic!("the literal is an object");
        };
        let created = Value::from_serde_json(serde_json::json!({
            "resourceType": "Observation",
            "id": "created-1"
        }));
        contain(&mut body, vec![created.clone()]);
        let document = Value::Object(body);
        assert_eq!(
            document
                .get("contained")
                .and_then(Value::as_array)
                .map(<[Value]>::to_vec),
            Some(vec![created])
        );
        assert_eq!(
            document
                .get("evidence")
                .and_then(|evidence| evidence.as_array()?.first())
                .and_then(|evidence| evidence.get("detail"))
                .and_then(|detail| detail.as_array()?.first())
                .and_then(|detail| detail.get("reference"))
                .and_then(Value::as_str),
            Some("#created-1")
        );
    }

    #[test]
    fn the_source_url_names_the_ehr_and_the_exact_version() {
        let base: url::Url = "http://cdr.invalid/openehr/v1/"
            .parse()
            .expect("a legal base URL");
        let ehr = EhrId::new("bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11").expect("a legal ehr_id");
        let version = ObjectVersionId::new("8849182c-82ad-4088-a07f-48ead4180515::ferroehr::2")
            .expect("a legal version id");
        let built = composition_url(&base, &ehr, &version).expect("the URL builds");
        assert_eq!(
            "http://cdr.invalid/openehr/v1/ehr/bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11/composition/8849182c-82ad-4088-a07f-48ead4180515::ferroehr::2",
            built.as_str()
        );
    }
}
