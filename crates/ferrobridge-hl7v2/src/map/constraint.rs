// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR R4 constraints a value and an element are held to before they
//! enter the Bundle.
//!
//! Two constraints decide whether a resource decodes: the lexical form of
//! each primitive (<https://hl7.org/fhir/R4/datatypes.html#primitive>, `url`
//! among them at <https://hl7.org/fhir/R4/datatypes.html#url>) and the
//! minimum cardinality of each element
//! (<https://hl7.org/fhir/R4/elementdefinition.html>, `ElementDefinition.min`).
//! Both are read from `fhir-types`: the lexical forms through its decoder, the
//! cardinalities from its element table. Nothing here names a resource or an
//! element.

use fhir_types::codec::{DecodeErrorKind, Json, Object, Path, Value};
use fhir_types::r4::extension::Extension;
use fhir_types::r4::schema::SCHEMAS;
use fhir_types::schema::{FieldSchema, Kind, TypeSchema};

/// The url of the extension a lexical probe decodes, itself a valid `uri`.
const PROBE_URL: &str = "urn:ferrobridge:lexical-probe";

/// Checks `value` against the lexical form of the FHIR primitive `code`.
///
/// A code `Extension.value[x]` does not carry (`xhtml`) is not checked here.
///
/// # Errors
///
/// Returns the decoder's refusal when the value is outside the primitive's
/// lexical form or of the wrong JSON kind.
pub(super) fn lexical(code: &str, value: &Value) -> Result<(), DecodeErrorKind> {
    let mut characters = code.chars();
    let Some(first) = characters.next() else {
        return Ok(());
    };
    let key = format!("value{}{}", first.to_ascii_uppercase(), characters.as_str());
    // NOTE: HL7 R4 extensibility §Extension: `value[x]` is an open type admitting
    // every primitive, so its decoder holds each primitive's lexical form.
    if SCHEMAS.element(&format!("Extension.{key}")).is_none() {
        return Ok(());
    }
    let mut probe = Object::new();
    probe.insert(String::from("url"), Value::String(String::from(PROBE_URL)));
    probe.insert(key, value.clone());
    match Extension::from_json(&probe, &mut Path::root("Extension")) {
        Ok(_) => Ok(()),
        Err(error) => Err(error.kind),
    }
}

/// An element dropped because it lacks an element its definition requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Dropped {
    /// The instance path of the dropped element, for example
    /// `MessageHeader.destination[0]`.
    pub(super) element: String,
    /// The definition path of the required element it lacks, for example
    /// `MessageHeader.destination.endpoint`.
    pub(super) required: &'static str,
}

/// Drops from `object`, of the type `schema` and at the instance path `at`,
/// every element that lacks one its definition requires, innermost first.
///
/// An element emptied by what it lost is removed with it, since FHIR JSON
/// holds no empty object or array (<https://hl7.org/fhir/R4/json.html>).
/// Returns the definition path of the first required element `object` itself
/// lacks once its elements are pruned, `None` when it lacks none.
pub(super) fn prune(
    object: &mut Object,
    schema: &'static TypeSchema,
    at: &str,
    dropped: &mut Vec<Dropped>,
) -> Option<&'static str> {
    for field in schema.fields {
        match field.kind {
            Kind::Complex(name) => descend(object, field.name, name, at, dropped),
            Kind::Primitive(_) => {
                descend(object, &format!("_{}", field.name), "Element", at, dropped);
            }
            Kind::Choice(variants) => {
                for (suffix, kind) in variants {
                    let key = format!("{}{suffix}", field.name);
                    match kind {
                        Kind::Complex(name) => descend(object, &key, name, at, dropped),
                        Kind::Primitive(_) => {
                            descend(object, &format!("_{key}"), "Element", at, dropped);
                        }
                        _ => {}
                    }
                }
            }
            Kind::Attribute | Kind::Resource | Kind::Xhtml => {}
        }
    }
    schema
        .fields
        .iter()
        .find(|field| field.min > 0 && !present(object, field))
        .map(|field| field.path)
}

/// Whether `object` carries `field`: its member, for a choice the member of
/// any alternative, and for a primitive also its `_` sibling with an
/// extension.
///
/// A primitive with extensions and no value is represented by the `_name`
/// property alone (<https://hl7.org/fhir/R4/json.html#primitive>), and it is
/// present for the element's cardinality.
fn present(object: &Object, field: &FieldSchema) -> bool {
    let carried = |key: &str, kind: Kind| {
        object.contains_key(key)
            || (matches!(kind, Kind::Primitive(_)) && extended(object.get(&format!("_{key}"))))
    };
    match field.kind {
        Kind::Choice(variants) => variants
            .iter()
            .any(|(suffix, kind)| carried(&format!("{}{suffix}", field.name), *kind)),
        kind => carried(field.name, kind),
    }
}

/// Whether a primitive's `_` sibling, one object or an array of them, holds
/// at least one extension.
fn extended(sibling: Option<&Value>) -> bool {
    let holds = |value: &Value| {
        value
            .get("extension")
            .and_then(Value::as_array)
            .is_some_and(|extensions| !extensions.is_empty())
    };
    match sibling {
        Some(Value::Array(items)) => items.iter().any(holds),
        Some(value) => holds(value),
        None => false,
    }
}

/// Prunes the member `key` of `object`, of the complex type `type_name`,
/// dropping each occurrence that lacks a required element.
///
/// A dropped occurrence of a primitive's `_` sibling becomes `null`, so the
/// sibling array stays aligned with the values
/// (<https://hl7.org/fhir/R4/json.html>, §Primitive Types).
fn descend(object: &mut Object, key: &str, type_name: &str, at: &str, dropped: &mut Vec<Dropped>) {
    let Some(schema) = SCHEMAS.type_named(type_name) else {
        return;
    };
    let Some(value) = object.get_mut(key) else {
        return;
    };
    let remove = match value {
        Value::Object(child) => {
            let path = format!("{at}.{key}");
            match prune(child, schema, &path, dropped) {
                Some(required) => {
                    dropped.push(Dropped {
                        element: path,
                        required,
                    });
                    true
                }
                None => child.is_empty(),
            }
        }
        Value::Array(items) => {
            let taken = core::mem::take(items);
            for (index, mut item) in taken.into_iter().enumerate() {
                let keep = match &mut item {
                    Value::Object(child) => {
                        let path = format!("{at}.{key}[{index}]");
                        match prune(child, schema, &path, dropped) {
                            Some(required) => {
                                dropped.push(Dropped {
                                    element: path,
                                    required,
                                });
                                false
                            }
                            None => !child.is_empty(),
                        }
                    }
                    _ => true,
                };
                if keep {
                    items.push(item);
                } else if key.starts_with('_') {
                    items.push(Value::Null);
                }
            }
            items.iter().all(Value::is_null)
        }
        _ => false,
    };
    if remove {
        object.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::{Dropped, lexical, prune};
    use fhir_types::codec::{DecodeErrorKind, Json, Object, Path, Value};
    use fhir_types::r4::resource::Resource;
    use fhir_types::r4::schema::SCHEMAS;

    fn object(json: &serde_json::Value) -> Object {
        match Value::from_serde_json(json.clone()) {
            Value::Object(object) => object,
            other => panic!("not an object: {other:?}"),
        }
    }

    /// Prunes `json` as a resource and returns what it holds afterwards.
    fn pruned(json: &serde_json::Value) -> (Object, Vec<Dropped>, Option<&'static str>) {
        let mut document = object(json);
        let resource_type = document
            .get("resourceType")
            .and_then(Value::as_str)
            .map(String::from)
            .expect("a resource type");
        let schema = SCHEMAS.type_named(&resource_type).expect("a resource");
        let mut dropped = Vec::new();
        let incomplete = prune(&mut document, schema, &resource_type, &mut dropped);
        (document, dropped, incomplete)
    }

    fn decodes(document: &Object) -> bool {
        let resource_type = document
            .get("resourceType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        Resource::from_json(document, &mut Path::root(resource_type)).is_ok()
    }

    fn text(value: &str) -> Value {
        Value::String(String::from(value))
    }

    #[test]
    fn an_application_name_with_spaces_is_no_url() {
        assert_eq!(
            lexical("url", &text("North Lab App")),
            Err(DecodeErrorKind::BadValue)
        );
        assert_eq!(lexical("url", &text("urn:oid:1.2.3")), Ok(()));
        assert_eq!(lexical("string", &text("North Lab App")), Ok(()));
        assert_eq!(
            lexical("date", &text("1980-00-00")),
            Err(DecodeErrorKind::BadValue)
        );
        assert_eq!(lexical("xhtml", &text("<div/>")), Ok(()));
    }

    #[test]
    fn a_destination_without_its_endpoint_is_dropped() {
        let (document, dropped, incomplete) = pruned(&serde_json::json!({
            "resourceType": "MessageHeader",
            "eventCoding": {"code": "R01"},
            "source": {"endpoint": "urn:oid:1.2.3"},
            "destination": [{"name": "EHR"}, {"endpoint": "urn:oid:1.2.4"}],
        }));
        assert_eq!(incomplete, None);
        assert_eq!(
            dropped,
            vec![Dropped {
                element: String::from("MessageHeader.destination[0]"),
                required: "MessageHeader.destination.endpoint",
            }]
        );
        assert_eq!(
            document
                .get("destination")
                .and_then(Value::as_array)
                .map(<[Value]>::len),
            Some(1)
        );
        assert!(decodes(&document));
    }

    #[test]
    fn an_address_extension_without_its_url_is_dropped() {
        let (document, dropped, incomplete) = pruned(&serde_json::json!({
            "resourceType": "Patient",
            "address": [{"city": "Northtown", "extension": [{"valueCode": "H"}]}],
        }));
        assert_eq!(incomplete, None);
        assert_eq!(
            dropped,
            vec![Dropped {
                element: String::from("Patient.address[0].extension[0]"),
                required: "Extension.url",
            }]
        );
        assert!(document.contains_key("address"));
        assert!(decodes(&document));
    }

    #[test]
    fn a_provenance_agent_without_who_is_dropped_and_leaves_the_provenance_incomplete() {
        let (_, dropped, incomplete) = pruned(&serde_json::json!({
            "resourceType": "Provenance",
            "target": [{"reference": "urn:uuid:00000000-0000-0000-0000-000000000001"}],
            "recorded": "2026-09-25T14:30:00+02:00",
            "agent": [{"type": {"text": "author"}}],
        }));
        assert_eq!(
            dropped,
            vec![Dropped {
                element: String::from("Provenance.agent[0]"),
                required: "Provenance.agent.who",
            }]
        );
        assert_eq!(incomplete, Some("Provenance.agent"));
    }

    #[test]
    fn a_message_header_without_its_source_is_incomplete() {
        let (_, dropped, incomplete) = pruned(&serde_json::json!({
            "resourceType": "MessageHeader",
            "eventCoding": {"code": "R01"},
            "source": {"name": "North Lab App"},
        }));
        assert_eq!(
            dropped,
            vec![Dropped {
                element: String::from("MessageHeader.source"),
                required: "MessageHeader.source.endpoint",
            }]
        );
        assert_eq!(incomplete, Some("MessageHeader.source"));
    }

    // NOTE: HL7 R4 JSON §Primitive Types: a primitive with extensions and no value is the
    // `_name` property alone, so `_endpoint` holding a data-absent-reason is the endpoint.
    #[test]
    fn an_endpoint_carried_only_by_its_extension_is_present() {
        let (document, dropped, incomplete) = pruned(&serde_json::json!({
            "resourceType": "MessageHeader",
            "eventCoding": {"code": "R01"},
            "source": {"_endpoint": {"extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/data-absent-reason",
                "valueCode": "unknown",
            }]}},
        }));
        assert_eq!(incomplete, None);
        assert_eq!(dropped, Vec::new());
        assert!(
            document
                .get("source")
                .and_then(|source| source.get("_endpoint"))
                .is_some(),
            "{document:?}"
        );
        assert!(decodes(&document));
    }

    #[test]
    fn an_endpoint_sibling_with_no_extension_is_missing() {
        let (_, dropped, incomplete) = pruned(&serde_json::json!({
            "resourceType": "MessageHeader",
            "eventCoding": {"code": "R01"},
            "source": {"name": "North Lab App", "_endpoint": {"extension": []}},
        }));
        assert_eq!(
            dropped,
            vec![Dropped {
                element: String::from("MessageHeader.source"),
                required: "MessageHeader.source.endpoint",
            }]
        );
        assert_eq!(incomplete, Some("MessageHeader.source"));
    }

    #[test]
    fn a_dropped_primitive_extension_keeps_the_sibling_array_aligned() {
        let (document, dropped, incomplete) = pruned(&serde_json::json!({
            "resourceType": "Patient",
            "name": [{
                "given": ["Ann", "Bea"],
                "_given": [{"extension": [{"url": "urn:x", "valueCode": "A"}]}, {"extension": [{"valueCode": "B"}]}],
            }],
        }));
        assert_eq!(incomplete, None);
        assert_eq!(dropped.len(), 1);
        let siblings = document
            .get("name")
            .and_then(Value::as_array)
            .and_then(<[Value]>::first)
            .and_then(|name| name.get("_given"))
            .and_then(Value::as_array)
            .map(|items| items.iter().map(Value::is_null).collect::<Vec<_>>());
        assert_eq!(siblings, Some(vec![false, true]));
        assert!(decodes(&document));
    }
}
