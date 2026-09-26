// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lowered legacy fields and the data types they name.

use std::collections::{BTreeMap, BTreeSet};

use fhir_codegen::v2::legacy::lower::{LegacyDefect, Owner};
use hl7v2_types::model::{DataTypeRef, LegacyBase, LegacyDataType};
use serde_json::Value;

use crate::v2_legacy::LEGACY;
use crate::v2_legacy::links::references;
use crate::v2_legacy::raw;

#[test]
fn lowered_legacy_fields_agree_with_the_field_and_data_element_tables() {
    for version in &LEGACY.versions {
        let fields = raw(&version.version, "fields");
        let elements: BTreeMap<String, Value> = raw(&version.version, "data_elements")
            .into_iter()
            .map(|element| (element["id"].as_str().expect("id").to_owned(), element))
            .collect();
        for (id, segment) in &version.segments {
            let mut rows: Vec<&Value> = fields
                .iter()
                .filter(|field| field["segment_id"] == id.as_str())
                .collect();
            rows.sort_by_key(|field| {
                field["position"]
                    .as_str()
                    .and_then(|text| text.parse::<u16>().ok())
            });
            let positions: BTreeSet<String> = rows
                .iter()
                .map(|row| row["position"].as_str().expect("a position").to_owned())
                .collect();
            let (listed, gaps): (Vec<_>, Vec<_>) = segment
                .fields
                .iter()
                .partition(|field| positions.contains(&field.position.to_string()));
            for gap in &gaps {
                assert!(
                    LegacyDefect::MissingField
                        .is_tolerated(&format!("{}/fields.json", version.version)),
                    "{} {}",
                    version.version,
                    gap.id
                );
                assert_eq!(
                    gap.optionality,
                    fhir_codegen::v2::lower::Optionality::Unstated
                );
                assert_eq!(gap.data_type, None);
            }
            assert_eq!(rows.len(), listed.len(), "{} {id}", version.version);
            for (row, field) in rows.iter().zip(listed) {
                let context = format!("{} {}", version.version, field.id);
                let element = &elements[row["data_element_id"].as_str().expect("an item")];
                assert_eq!(row["position"], field.position.to_string(), "{context}");
                assert_eq!(element["description"], field.name.as_str(), "{context}");
                let code = element["datatype_id"].as_str().expect("a data type");
                assert_eq!(
                    field.data_type.as_deref(),
                    (code != "-").then_some(code),
                    "{context}"
                );
                assert_eq!(
                    element["table_id"].as_str(),
                    field.table.as_ref().map(|table| table.id.as_str()),
                    "{context}"
                );
                assert_eq!(row["min"], field.cardinality.min.to_string(), "{context}");
            }
        }
    }
}

/// The legacy data type a field of `segment` at `position` names.
fn legacy_type(
    segment: &'static hl7v2_types::model::Segment,
    position: u16,
) -> &'static LegacyDataType {
    let field = segment
        .fields
        .iter()
        .find(|field| field.position == position)
        .expect("the field");
    match field.data_type {
        Some(DataTypeRef::Legacy(data_type)) => data_type,
        other => panic!("{}: expected a legacy data type, got {other:?}", field.id),
    }
}

#[test]
fn a_version_specific_code_links_the_base_type_it_stands_for() {
    let msh = &hl7v2_types::legacy::v2_3::segment::msh::MSH;
    let dg1 = &hl7v2_types::legacy::v2_3::segment::dg1::DG1;
    let message_type = legacy_type(msh, 9);
    assert_eq!(message_type.code, "CM_MSG");
    assert_eq!(
        message_type.base,
        Some(LegacyBase {
            code: "MSG",
            table: None
        })
    );
    let diagnosis = legacy_type(dg1, 3);
    assert_eq!(diagnosis.code, "CE_0051");
    assert_eq!(
        diagnosis.base,
        Some(LegacyBase {
            code: "CE",
            table: Some("0051")
        })
    );
    let stamp = legacy_type(msh, 7);
    assert_eq!(stamp.code, "TS");
    assert_eq!(
        stamp.base,
        Some(LegacyBase {
            code: "DTM",
            table: None
        })
    );
    assert_eq!(
        legacy_type(msh, 4).base,
        None,
        "HD stands for no other type"
    );
}

#[test]
fn every_legacy_field_type_is_its_versions_own_code_with_the_tables_name() {
    for version in &LEGACY.versions {
        let names: BTreeMap<String, String> = raw(&version.version, "datatypes")
            .into_iter()
            .map(|row| {
                (
                    row["id"].as_str().expect("id").to_owned(),
                    row["description"].as_str().expect("a name").to_owned(),
                )
            })
            .collect();
        for (code, data_type) in &version.data_types {
            assert_eq!(Some(&data_type.name), names.get(code), "{code}");
        }
    }
    // A segment identical to an earlier version's, with every data type it names agreeing
    // there, links to that version's static, so the type is the owner's with this version's name.
    for structure in &hl7v2_types::legacy::STRUCTURES {
        let lowered = LEGACY
            .versions
            .iter()
            .find(|version| version.version == structure.version)
            .expect("a lowered version");
        let names: BTreeMap<String, String> = raw(structure.version, "datatypes")
            .into_iter()
            .map(|row| {
                (
                    row["id"].as_str().expect("id").to_owned(),
                    row["description"].as_str().expect("a name").to_owned(),
                )
            })
            .collect();
        let mut segments = Vec::new();
        references(structure.nodes, &mut segments);
        for segment in segments.into_iter().filter(|segment| segment.url.is_none()) {
            let owner = match lowered.segment_owner(segment.id) {
                Owner::Version(owner) => owner,
                Owner::Current => panic!("{} links to v2.9.1 but has no url", segment.id),
            };
            for field in segment.fields {
                if let Some(DataTypeRef::Legacy(data_type)) = field.data_type {
                    assert_eq!(data_type.version, owner, "{}", field.id);
                    assert_eq!(
                        Some(data_type.name),
                        names.get(data_type.code).map(String::as_str),
                        "{} at {}",
                        field.id,
                        structure.version
                    );
                    if let Some(base) = data_type.base {
                        assert_ne!(base.code, data_type.code, "{}", field.id);
                    }
                }
            }
        }
    }
}

#[test]
fn the_stray_obx_4_requirement_is_emitted_conditional() {
    for version in ["2.7.1", "2.8"] {
        let raw_usage = raw(version, "fields")
            .into_iter()
            .find(|row| row["segment_id"] == "OBX" && row["position"] == "4")
            .expect("an OBX-4 row")["usage"]
            .clone();
        assert_eq!(raw_usage, "R", "{version}");
        let obx = LEGACY
            .versions
            .iter()
            .find(|lowered| lowered.version == version)
            .and_then(|lowered| lowered.segments.get("OBX"))
            .expect("the version emits its OBX");
        let field = obx
            .fields
            .iter()
            .find(|field| field.position == 4)
            .expect("OBX-4");
        assert_eq!(
            field.optionality,
            fhir_codegen::v2::lower::Optionality::C,
            "{version}"
        );
    }
}
