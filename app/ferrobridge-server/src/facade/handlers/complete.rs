// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Making a rendered resource a valid R4 instance before the facade answers
//! it.
//!
//! A mapping may carry no row for the resource's subject: the ingest reads the
//! subject to find the EHR, and nothing brings it back out of the
//! composition. The identity map knows the person the EHR was recorded for, so
//! the facade writes `subject` (or `patient`, where the type names it so) from
//! that binding whenever the rendered resource lacks it. Then every element
//! the `fhir-types` element table marks `min 1` is checked, at the top and
//! inside every element present, and a resource that still lacks one is
//! refused: a read never answers an instance R4 does not admit
//! (<https://hl7.org/fhir/R4/conformance-rules.html#cardinality>). No
//! specification governs the fill: our own design.

use fhir_types::codec::Object;
use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhir_types::schema::FieldSchema;
use fhir_types::schema::Kind;
use fhir_types::schema::TypeSchema;

use crate::facade::identity::PersonId;
use crate::facade::ingest::Refused;
use crate::facade::outcome::Issue;
use crate::facade::status;

/// The element names a resource names its subject by, in the order the fill
/// tries them.
const SUBJECT_ELEMENTS: [&str; 2] = ["subject", "patient"];

/// Returns the `Reference` a resource names `person` by.
///
/// The value is the inverse of how the ingest reads a subject, so a read sent
/// back as an update resolves to the same EHR: a person keyed by a literal
/// reference in the deployment's `subject_namespace` is that reference again,
/// and every other person is a logical reference by its identifier
/// (<https://hl7.org/fhir/R4/references.html#logical>).
#[must_use]
pub(crate) fn reference_to(person: &PersonId, subject_namespace: &str) -> Value {
    let literal = person.namespace() == subject_namespace && person.id().contains('/');
    let reference = if literal {
        serde_json::json!({ "reference": person.id() })
    } else {
        serde_json::json!({
            "type": "Patient",
            "identifier": { "system": person.namespace(), "value": person.id() }
        })
    };
    Value::from_serde_json(reference)
}

/// Writes `subject` into the element `resource_type` names its subject by,
/// when `body` lacks it, and returns the element path it filled.
///
/// The element is the first of `subject` and `patient` the element table
/// gives the type as a `Reference`; a type with neither is left as it is.
pub(crate) fn fill_subject(
    body: &mut Object,
    resource_type: &str,
    subject: Option<&Value>,
) -> Option<&'static str> {
    let subject = subject?;
    let schema = resource_schema(resource_type)?;
    let field = SUBJECT_ELEMENTS.iter().find_map(|name| {
        schema.fields.iter().find(|field| {
            field.name == *name && !field.many && matches!(field.kind, Kind::Complex("Reference"))
        })
    })?;
    if present(body, field) {
        return None;
    }
    body.insert(String::from(field.name), subject.clone());
    Some(field.path)
}

/// Returns the location of every required element `body` lacks, in document
/// order.
///
/// An element is required where the element table gives it `min 1`, at the
/// top of the resource and inside every complex element, backbone element and
/// contained resource the body carries. A primitive counts as present when
/// its value or its `_` sibling is (<https://hl7.org/fhir/R4/json.html#primitive>),
/// and a choice when any of its typed forms is.
#[must_use]
pub(crate) fn absent_required(body: &Object, resource_type: &str) -> Vec<String> {
    let mut absent = Vec::new();
    if let Some(schema) = resource_schema(resource_type) {
        walk(schema, body, resource_type, &mut absent);
    }
    absent
}

/// Returns the refusal a rendered resource lacking `absent` answers.
///
/// The issues name element paths and never a value, so the refusal carries no
/// clinical content. The status is the table's [`status::INTERNAL`] row: the
/// mapping and the binding are the bridge's own, and a client can change
/// nothing to clear it.
#[must_use]
pub(crate) fn refusal(resource_type: &str, absent: &[String]) -> Refused {
    let issues = absent
        .iter()
        .map(|location| {
            Issue::error(status::INTERNAL.issue())
                .diagnosing(format!(
                    "the {resource_type} this server rendered lacks {location}, which R4 requires, \
                     and neither the mapping nor the identity binding supplies it"
                ))
                .at(location.clone())
        })
        .collect();
    Refused::of(status::INTERNAL.status(), issues)
}

/// Returns the element table of the resource type `name`.
fn resource_schema(name: &str) -> Option<&'static TypeSchema> {
    if !SCHEMAS.is_resource(name) {
        return None;
    }
    SCHEMAS
        .type_named(name)
        .filter(|schema| schema.path == name)
}

/// Records every required element `object` of type `schema` lacks, and walks
/// into every element it carries.
fn walk(schema: &TypeSchema, object: &Object, at: &str, absent: &mut Vec<String>) {
    for field in schema.fields {
        if field.min > 0 && !present(object, field) {
            absent.push(format!("{at}.{}", field.name));
        }
        match field.kind {
            Kind::Choice(variants) => {
                for (suffix, kind) in variants {
                    let name = format!("{}{suffix}", field.name);
                    descend(*kind, object.get(&name), &format!("{at}.{name}"), absent);
                }
            }
            kind => descend(
                kind,
                object.get(field.name),
                &format!("{at}.{}", field.name),
                absent,
            ),
        }
    }
}

/// Walks into the value one element holds, each item of a repeating one at
/// its index.
fn descend(kind: Kind, value: Option<&Value>, at: &str, absent: &mut Vec<String>) {
    match value {
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                descend_one(kind, item, &format!("{at}[{index}]"), absent);
            }
        }
        Some(item) => descend_one(kind, item, at, absent),
        None => {}
    }
}

/// Walks into one complex value or one contained resource.
fn descend_one(kind: Kind, value: &Value, at: &str, absent: &mut Vec<String>) {
    let Value::Object(object) = value else {
        return;
    };
    let schema = match kind {
        Kind::Complex(name) => SCHEMAS.type_named(name),
        Kind::Resource => object
            .get("resourceType")
            .and_then(Value::as_str)
            .and_then(resource_schema),
        Kind::Attribute | Kind::Primitive(_) | Kind::Xhtml | Kind::Choice(_) => None,
    };
    if let Some(schema) = schema {
        walk(schema, object, at, absent);
    }
}

/// Whether `object` carries the element `field` describes.
fn present(object: &Object, field: &FieldSchema) -> bool {
    match field.kind {
        Kind::Choice(variants) => variants.iter().any(|(suffix, kind)| {
            let name = format!("{}{suffix}", field.name);
            carries(object, &name, matches!(kind, Kind::Primitive(_)))
        }),
        Kind::Primitive(_) => carries(object, field.name, true),
        Kind::Attribute | Kind::Complex(_) | Kind::Resource | Kind::Xhtml => {
            carries(object, field.name, false)
        }
    }
}

/// Whether `object` holds a value under `name`, or under `_name` for a
/// primitive.
fn carries(object: &Object, name: &str, primitive: bool) -> bool {
    if object.get(name).is_some_and(holds) {
        return true;
    }
    primitive && object.get(&format!("_{name}")).is_some_and(holds)
}

/// Whether `value` holds anything: JSON `null` and an array of nothing but
/// `null` hold nothing (<https://hl7.org/fhir/R4/json.html>).
fn holds(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Array(items) => items.iter().any(holds),
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{absent_required, fill_subject, reference_to, refusal};
    use crate::facade::identity::PersonId;
    use fhir_types::codec::Object;
    use fhir_types::codec::Value;
    use http::StatusCode;

    /// A synthetic value no refusal may carry onto the wire.
    const MARKER: &str = "ferrobridge-synthetic-clinical-marker";

    /// Returns `literal` as a resource object.
    fn object(literal: serde_json::Value) -> Object {
        let Value::Object(object) = Value::from_serde_json(literal) else {
            panic!("the literal is an object");
        };
        object
    }

    #[test]
    fn a_condition_without_subject_lacks_the_one_element_r4_requires() {
        // R4 condition.html: Condition.subject is 1..1.
        let body = object(serde_json::json!({
            "resourceType": "Condition",
            "code": { "text": MARKER }
        }));
        assert_eq!(
            vec![String::from("Condition.subject")],
            absent_required(&body, "Condition")
        );
    }

    #[test]
    fn the_subject_is_filled_from_the_binding_and_the_resource_is_then_complete() {
        let mut body = object(serde_json::json!({ "resourceType": "Condition" }));
        let person = PersonId::new("http://example.org/fhir/sid/mrn", "p-1").expect("a person");
        let subject = reference_to(&person, "ferrobridge");
        assert_eq!(
            Some("Condition.subject"),
            fill_subject(&mut body, "Condition", Some(&subject))
        );
        assert_eq!(Some(&subject), body.get("subject"));
        assert!(absent_required(&body, "Condition").is_empty());
    }

    #[test]
    fn a_subject_the_mapping_wrote_is_kept() {
        let mapped = serde_json::json!({ "reference": "Patient/mapped" });
        let mut body = object(serde_json::json!({
            "resourceType": "Condition",
            "subject": mapped
        }));
        let person = PersonId::new("ferrobridge", "Patient/bound").expect("a person");
        let subject = reference_to(&person, "ferrobridge");
        assert_eq!(None, fill_subject(&mut body, "Condition", Some(&subject)));
        assert_eq!(Some(&Value::from_serde_json(mapped)), body.get("subject"));
    }

    #[test]
    fn a_type_naming_its_subject_patient_is_filled_there() {
        // R4 allergyintolerance.html: AllergyIntolerance.patient is 1..1.
        let mut body = object(serde_json::json!({ "resourceType": "AllergyIntolerance" }));
        let person = PersonId::new("ferrobridge", "Patient/p-1").expect("a person");
        let subject = reference_to(&person, "ferrobridge");
        assert_eq!(
            Some("AllergyIntolerance.patient"),
            fill_subject(&mut body, "AllergyIntolerance", Some(&subject))
        );
        assert_eq!(
            Some("Patient/p-1"),
            body.get("patient")
                .and_then(|patient| patient.get("reference"))
                .and_then(Value::as_str)
        );
    }

    #[test]
    fn a_literal_reference_in_the_deployment_namespace_renders_as_that_reference() {
        let literal = PersonId::new("ferrobridge", "Patient/p-1").expect("a person");
        assert_eq!(
            Value::from_serde_json(serde_json::json!({ "reference": "Patient/p-1" })),
            reference_to(&literal, "ferrobridge")
        );
        let logical = PersonId::new("http://example.org/fhir/sid/mrn", "p-1").expect("a person");
        assert_eq!(
            Value::from_serde_json(serde_json::json!({
                "type": "Patient",
                "identifier": { "system": "http://example.org/fhir/sid/mrn", "value": "p-1" }
            })),
            reference_to(&logical, "ferrobridge")
        );
    }

    #[test]
    fn a_required_element_inside_a_present_element_is_checked() {
        // R4 observation.html: Observation.status and Observation.code are
        // 1..1, and so is Observation.component.code.
        let body = object(serde_json::json!({
            "resourceType": "Observation",
            "status": "final",
            "code": { "text": "synthetic" },
            "component": [{ "valueString": MARKER }]
        }));
        assert_eq!(
            vec![String::from("Observation.component[0].code")],
            absent_required(&body, "Observation")
        );
    }

    #[test]
    fn a_primitive_carried_only_by_its_extension_sibling_is_present() {
        let body = object(serde_json::json!({
            "resourceType": "Observation",
            "_status": { "extension": [{ "url": "http://example.org/x", "valueString": "y" }] },
            "code": { "text": "synthetic" }
        }));
        assert!(absent_required(&body, "Observation").is_empty());
    }

    #[test]
    fn a_contained_resource_is_checked_as_its_own_type() {
        let body = object(serde_json::json!({
            "resourceType": "Condition",
            "subject": { "reference": "Patient/p-1" },
            "contained": [{ "resourceType": "Observation", "id": "c1", "status": "final" }]
        }));
        assert_eq!(
            vec![String::from("Condition.contained[0].code")],
            absent_required(&body, "Condition")
        );
    }

    #[tokio::test]
    async fn a_required_element_the_binding_cannot_fill_is_refused_by_its_path_alone() {
        // A Condition whose EHR has no recorded person keeps lacking subject.
        let mut body = object(serde_json::json!({
            "resourceType": "Condition",
            "code": { "text": MARKER }
        }));
        assert_eq!(None, fill_subject(&mut body, "Condition", None));
        let absent = absent_required(&body, "Condition");
        let refused = refusal("Condition", &absent);
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, refused.status());
        let response = crate::facade::reply::Refusal::from(refused).into_response();
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, response.status());
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("the refusal body reads");
        let text = String::from_utf8(bytes.to_vec()).expect("the body is UTF-8");
        let outcome: serde_json::Value = serde_json::from_str(&text).expect("an outcome");
        assert_eq!(
            Some("Condition.subject"),
            outcome["issue"][0]["location"][0].as_str(),
            "{text}"
        );
        assert!(!text.contains(MARKER), "the body reached the wire: {text}");
    }
}
