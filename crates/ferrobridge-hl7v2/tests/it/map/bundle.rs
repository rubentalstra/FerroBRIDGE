// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Bundles the fixtures map to, decoded as R4 and reviewed as snapshots.

use fhir_types::codec::{Json, Path, Value};

use crate::fixtures;

use crate::map::{entries, mapped, pretty, resources, summary};

#[tokio::test]
async fn pid_maps_to_a_patient_that_decodes_as_r4() {
    let (mapped, _) = mapped(&fixtures::oru_r01()).await;
    let patients = resources(mapped.bundle(), "Patient");
    assert_eq!(patients.len(), 1, "one PID, one Patient");
    let Value::Object(object) = patients[0] else {
        panic!("a Patient object");
    };
    let patient = fhir_types::r4::patient::Patient::from_json(object, &mut Path::root("Patient"))
        .expect("the Patient decodes as R4");
    assert_eq!(patient.name.len(), 1, "PID-5 is one name");
    assert_eq!(
        patient.name[0]
            .family
            .as_ref()
            .and_then(|family| family.value.as_deref()),
        Some("M\u{FC}ller"),
        "the Latin-1 family name arrives as text"
    );
    assert_eq!(
        patient
            .gender
            .as_ref()
            .and_then(|gender| gender.value.as_deref()),
        Some("male"),
        "PID-8 through table-hl70001-to-administrative-gender"
    );
    assert_eq!(
        patient
            .birth_date
            .as_ref()
            .and_then(|date| date.value.as_deref()),
        Some("1980-01-01")
    );
}

#[tokio::test]
async fn each_obx_maps_to_an_observation_that_decodes_as_r4() {
    let (mapped, _) = mapped(&fixtures::oru_r01()).await;
    let observations = resources(mapped.bundle(), "Observation");
    assert_eq!(observations.len(), 2, "two OBX, two Observations");
    for (value, expected) in observations.iter().zip(["5.4", "140"]) {
        let Value::Object(object) = value else {
            panic!("an Observation object");
        };
        let observation = fhir_types::r4::observation::Observation::from_json(
            object,
            &mut Path::root("Observation"),
        )
        .expect("the Observation decodes as R4");
        assert_eq!(
            observation.status.value.as_deref(),
            Some("final"),
            "OBX-11 through table-hl70085-to-observation-status"
        );
        let written = value
            .get("valueQuantity")
            .and_then(|quantity| quantity.get("value"));
        assert!(
            matches!(written, Some(Value::Number(number)) if number.as_str() == expected),
            "OBX-5 as the quantity value: {written:?}"
        );
    }
}

#[tokio::test]
async fn the_bundle_is_a_message_with_the_message_header_first() {
    let (mapped, _) = mapped(&fixtures::oru_r01()).await;
    let bundle = mapped.bundle();
    assert_eq!(bundle.get("type").and_then(Value::as_str), Some("message"));
    let first = entries(bundle)
        .first()
        .and_then(|entry| entry.get("resource"))
        .and_then(|resource| resource.get("resourceType"))
        .and_then(Value::as_str);
    assert_eq!(first, Some("MessageHeader"));
    for entry in entries(bundle) {
        assert!(
            entry
                .get("fullUrl")
                .and_then(Value::as_str)
                .is_some_and(|url| url.starts_with("urn:uuid:")),
            "every entry has a urn:uuid full url"
        );
    }
}

#[tokio::test]
async fn the_oru_r01_maps_to_the_reviewed_bundle() {
    let (mapped, asked) = mapped(&fixtures::oru_r01()).await;
    insta::assert_snapshot!("oru_r01_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!("oru_r01_outcomes", summary(&mapped));
    insta::assert_debug_snapshot!("oru_r01_counts", mapped.counts());
    insta::assert_debug_snapshot!("oru_r01_translations", asked);
}

#[tokio::test]
async fn the_adt_a01_maps_to_the_reviewed_bundle() {
    let (mapped, asked) = mapped(&fixtures::adt_a01()).await;
    insta::assert_snapshot!("adt_a01_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!("adt_a01_outcomes", summary(&mapped));
    insta::assert_debug_snapshot!("adt_a01_translations", asked);
}

#[tokio::test]
async fn the_mdm_t02_maps_to_the_reviewed_bundle() {
    let (mapped, asked) = mapped(&fixtures::mdm_t02()).await;
    insta::assert_snapshot!("mdm_t02_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!("mdm_t02_outcomes", summary(&mapped));
    insta::assert_debug_snapshot!("mdm_t02_translations", asked);
}
