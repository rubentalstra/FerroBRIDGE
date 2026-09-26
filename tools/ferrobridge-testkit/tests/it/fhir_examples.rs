// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HL7 examples package of each FHIR version through `fhir-types`.
//!
//! Every example resource decodes through the strict JSON codec of its
//! version, re-encodes, and then travels through the XML codec and back. A
//! case passes when both round trips give back the document the package
//! published, compared as the lexical document model, whose objects are
//! ordered maps and whose numbers keep their text, so the comparison is
//! byte equality of the canonical form. The first refusal or difference is
//! the failure reason. No specification governs the instrument: our own
//! design; the codecs answer to <https://hl7.org/fhir/json.html> and
//! <https://hl7.org/fhir/xml.html> of each version.

use std::error::Error;
use std::path::Path;

use ferrobridge_testkit::conformance::Case;
use ferrobridge_testkit::conformance::Corpus;
use ferrobridge_testkit::conformance::record;
use ferrobridge_testkit::examples::Package;
use fhir_types::codec::Json;
use fhir_types::codec::Path as ElementPath;
use fhir_types::codec::Value;
use fhir_types::codec::expect_object;
use fhir_types::schema::Schemas;
use fhir_types::xml::from_xml;
use fhir_types::xml::to_xml;

/// Returns why `file` fails its round trips, or `None` when both hold.
fn failure<T: Json>(schemas: &Schemas, file: &Path) -> Option<String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) => return Some(format!("unreadable: {error}")),
    };
    let original: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => return Some(format!("not JSON: {error}")),
    };
    let mut path = ElementPath::root("Resource");
    let decoded = match expect_object(&original, &path).and_then(|o| T::from_json(o, &mut path)) {
        Ok(decoded) => decoded,
        Err(error) => return Some(format!("JSON decode: {error}")),
    };
    let encoded = match decoded.to_json() {
        Ok(object) => Value::Object(object),
        Err(error) => return Some(format!("JSON encode: {error}")),
    };
    if let Some(at) = difference(&original, &encoded, "") {
        return Some(format!("the JSON round trip differs at {at}"));
    }
    let Value::Object(ref object) = encoded else {
        return Some(String::from("the JSON encoder gave back no object"));
    };
    let xml = match to_xml(schemas, object) {
        Ok(xml) => xml,
        Err(error) => return Some(format!("XML encode: {error}")),
    };
    let back = match from_xml(schemas, &xml) {
        Ok(back) => Value::Object(back),
        Err(error) => return Some(format!("XML decode: {error}")),
    };
    difference(&encoded, &back, "").map(|at| format!("the XML round trip differs at {at}"))
}

/// Returns the first place `after` departs from `before`, by element path.
fn difference(before: &Value, after: &Value, path: &str) -> Option<String> {
    match (before, after) {
        (Value::Object(was), Value::Object(now)) => {
            for (key, value) in was {
                let next = member(path, key);
                match now.get(key) {
                    None => return Some(format!("{next} (dropped)")),
                    Some(other) => {
                        if let Some(found) = difference(value, other, &next) {
                            return Some(found);
                        }
                    }
                }
            }
            now.keys()
                .find(|key| !was.contains_key(*key))
                .map(|key| format!("{} (added)", member(path, key)))
        }
        (Value::Array(was), Value::Array(now)) => {
            for (index, (value, other)) in was.iter().zip(now).enumerate() {
                if let Some(found) = difference(value, other, &format!("{path}[{index}]")) {
                    return Some(found);
                }
            }
            (was.len() != now.len())
                .then(|| format!("{path} ({} items became {})", was.len(), now.len()))
        }
        _ => (before != after).then(|| format!("{path} (value)")),
    }
}

/// Returns the path of the member `key` below `path`.
fn member(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// Whether `package` is on disk, or why its corpus skips.
#[expect(
    clippy::print_stderr,
    reason = "a corpus whose build-time package is absent says it skipped"
)]
fn fetched(package: Package) -> bool {
    let present = package.fetched();
    if !present {
        eprintln!(
            "skipped: {package} is fetched at build time; run scripts/vendor/fhir-packages.sh --build-time"
        );
    }
    present
}

/// Runs every example of `package` and records the verdicts as `corpus`.
fn measure<T: Json>(
    package: Package,
    corpus: Corpus,
    schemas: &Schemas,
) -> Result<(), Box<dyn Error>> {
    if !fetched(package) {
        return Ok(());
    }
    let files = package.resources()?;
    assert!(
        files.len() > 2000,
        "{package} holds {} example resources",
        files.len()
    );
    let mut cases = Vec::with_capacity(files.len());
    for file in &files {
        let id = file
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or("an example file has a UTF-8 name")?;
        cases.push(match failure::<T>(schemas, file) {
            None => Case::pass(id),
            Some(reason) => Case::fail(id, reason),
        });
    }
    let outcome = record(corpus, &cases)?;
    assert!(
        outcome.regressed.is_empty(),
        "cases the {corpus} pass list records no longer pass: {:?}",
        outcome.regressed
    );
    Ok(())
}

#[test]
fn conformance_the_fhir_r4_examples_hold_their_pass_list() -> Result<(), Box<dyn Error>> {
    measure::<fhir_types::r4::resource::Resource>(
        Package::R4,
        Corpus::FhirR4,
        &fhir_types::r4::schema::SCHEMAS,
    )
}

#[test]
fn conformance_the_fhir_r4b_examples_hold_their_pass_list() -> Result<(), Box<dyn Error>> {
    measure::<fhir_types::r4b::resource::Resource>(
        Package::R4b,
        Corpus::FhirR4b,
        &fhir_types::r4b::schema::SCHEMAS,
    )
}

#[test]
fn conformance_the_fhir_r5_examples_hold_their_pass_list() -> Result<(), Box<dyn Error>> {
    measure::<fhir_types::r5::resource::Resource>(
        Package::R5,
        Corpus::FhirR5,
        &fhir_types::r5::schema::SCHEMAS,
    )
}

#[test]
fn conformance_the_fhir_r6_examples_hold_their_pass_list() -> Result<(), Box<dyn Error>> {
    measure::<fhir_types::r6::resource::Resource>(
        Package::R6,
        Corpus::FhirR6,
        &fhir_types::r6::schema::SCHEMAS,
    )
}

#[test]
fn a_difference_names_the_first_element_that_moved() -> Result<(), Box<dyn Error>> {
    let before: Value = serde_json::from_str(r#"{"a":[{"b":"x"},{"b":"y"}],"c":1.0}"#)?;
    let changed: Value = serde_json::from_str(r#"{"a":[{"b":"x"},{"b":"z"}],"c":1.0}"#)?;
    let reprinted: Value = serde_json::from_str(r#"{"a":[{"b":"x"},{"b":"y"}],"c":1.00}"#)?;
    let reordered: Value = serde_json::from_str(r#"{"c":1.0,"a":[{"b":"x"},{"b":"y"}]}"#)?;
    let added: Value = serde_json::from_str(r#"{"a":[{"b":"x"},{"b":"y"}],"c":1.0,"d":true}"#)?;
    assert_eq!(
        difference(&before, &changed, "").as_deref(),
        Some("a[1].b (value)")
    );
    assert_eq!(
        difference(&before, &reprinted, "").as_deref(),
        Some("c (value)"),
        "a number keeps its text"
    );
    assert_eq!(
        difference(&before, &reordered, ""),
        None,
        "member order is not content"
    );
    assert_eq!(
        difference(&before, &added, "").as_deref(),
        Some("d (added)")
    );
    Ok(())
}
