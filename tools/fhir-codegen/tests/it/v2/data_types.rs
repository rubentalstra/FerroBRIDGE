// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data types and their components against the definitions.

use fhir_codegen::roots::{V2_COMPLEX_DIR, V2_DATA_TYPE_DIRS, V2_PRIMITIVE_DIR};
use hl7v2_types::model::{ConformanceLength, Length};
use serde_json::Value;

use crate::v2::MODEL;
use crate::v2::elements;
use crate::v2::extension;
use crate::v2::json_files;
use crate::v2::nested;
use crate::v2::optionality_code;
use crate::v2::raw;
use crate::v2::raw_cardinality;
use crate::v2::raw_integer;

fn raw_bool_or_integer(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        other => match raw_integer(other) {
            Some(1) => Some(true),
            Some(0) => Some(false),
            _ => None,
        },
    }
}

/// Compares the `length` and `conformance-length` extensions of `element` with
/// the emitted ones; `noTruncate` is `1`/`0` on a field and `true`/`false` on a component.
pub(super) fn assert_lengths(
    element: &Value,
    length: Option<Length>,
    conformance_length: Option<ConformanceLength>,
    context: &str,
) {
    let raw = extension(element, "http://hl7.org/v2/StructureDefinition/length");
    assert_eq!(raw.is_some(), length.is_some(), "{context}");
    if let (Some(raw), Some(emitted)) = (raw, length) {
        assert_eq!(
            nested(raw, "min").and_then(raw_integer),
            Some(u64::from(emitted.min))
        );
        assert_eq!(
            nested(raw, "max").and_then(raw_integer),
            emitted.max.map(u64::from)
        );
    }
    let conformance = extension(
        element,
        "http://hl7.org/v2/StructureDefinition/conformance-length",
    );
    assert_eq!(
        conformance.is_some(),
        conformance_length.is_some(),
        "{context}"
    );
    if let (Some(conformance), Some(emitted)) = (conformance, conformance_length) {
        assert_eq!(
            nested(conformance, "length").and_then(raw_integer),
            emitted.length.map(u64::from),
            "{context}"
        );
        let no_truncate = conformance["extension"]
            .as_array()
            .and_then(|nested| nested.iter().find(|entry| entry["url"] == "noTruncate"))
            .and_then(|entry| {
                raw_bool_or_integer(if entry["valueBoolean"].is_null() {
                    &entry["valueInteger"]
                } else {
                    &entry["valueBoolean"]
                })
            });
        assert_eq!(no_truncate, emitted.no_truncate, "{context}");
    }
}

#[test]
fn every_data_type_file_is_emitted_and_its_components_agree() {
    let mut components = 0;
    let mut primitives = 0;
    let files: Vec<String> = V2_DATA_TYPE_DIRS
        .iter()
        .flat_map(|dir| json_files(dir))
        .collect();
    assert_eq!(files.len(), 83);
    assert_eq!(hl7v2_types::data_type::DATA_TYPES.len(), files.len());
    for file in &files {
        let definition = raw(file);
        let code = definition["id"].as_str().expect("id");
        let emitted = hl7v2_types::data_type::find(code).expect("every data type is emitted");
        assert_eq!(definition["url"], emitted.url);
        let elements = elements(&definition);
        let (root, raw_components) = elements.split_first().expect("a root element");
        assert_eq!(root["short"], emitted.name);
        assert_eq!(raw_components.len(), emitted.components.len(), "{code}");
        if file.starts_with(V2_PRIMITIVE_DIR) {
            primitives += 1;
            assert!(emitted.components.is_empty(), "{code} is primitive");
        } else {
            assert!(file.starts_with(V2_COMPLEX_DIR));
        }
        for (index, (element, component)) in
            raw_components.iter().zip(emitted.components).enumerate()
        {
            components += 1;
            let context = &component.id;
            assert_eq!(element["id"], component.id);
            assert_eq!(usize::from(component.position), index + 1, "{context}");
            assert_eq!(element["short"], component.name, "{context}");
            assert_eq!(
                element["type"][0]["code"].as_str().map(str::to_owned),
                component.data_type.map(|data_type| format!(
                    "http://hl7.org/v2/StructureDefinition/{}",
                    data_type.code
                )),
                "{context}"
            );
            if element["min"].is_null() {
                assert_eq!(component.cardinality, None, "{context}");
            } else {
                assert_eq!(
                    Some(raw_cardinality(element)),
                    component.cardinality,
                    "{context}"
                );
            }
            let optionality =
                extension(element, "http://hl7.org/v2/StructureDefinition/optionality")
                    .expect("optionality");
            assert_eq!(
                optionality["valueCode"].as_str(),
                Some(optionality_code(component.optionality).as_str()),
                "{context}"
            );
            assert_lengths(
                element,
                component.length,
                component.conformance_length,
                context,
            );
            assert_eq!(
                element["binding"]["valueSet"].as_str(),
                component.table.map(|table| table.value_set),
                "{context}"
            );
        }
    }
    assert_eq!(primitives, 12);
    assert_eq!(components, 448);
    assert_eq!(components, MODEL.component_count());
}
