// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two FHIRconnect operations, against the DRAFT REST API chapter.
//!
//! Every wire case here reads the chapter at its pinned commit, an unmerged
//! draft (pull request #93 of the specification), so the fixtures under
//! `tests/fixtures/draft-rest-api/` are the chapter's own worked examples and
//! the expectations move with it when it merges.

mod definitions;
mod examples;
mod runs;

use core::error::Error;

use fhir_types::codec::Json;
use fhir_types::codec::Path;
use fhir_types::r4::bundle::Bundle;
use fhir_types::r4::bundle::BundleEntry;
use fhir_types::r4::parameters::Parameters;
use fhir_types::r4::primitives::Code;
use fhir_types::r4::reference::Reference;
use fhir_types::r4::resource::Resource;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::traverse::functions::NoMappingFunctions;
use fhirconnect::operations::contract::ToOpenehrRequest;
use fhirconnect::operations::programs::ProgramSet;
use fhirconnect::operations::run;
use fhirconnect::operations::run::Settings;

use crate::support::FIXTURES;
use crate::support::compiled;
use crate::support::template;

/// The profile the minimal diagnosis context claims.
const MINIMAL_PROFILE: &str =
    "http://example.org/fhir/StructureDefinition/ferrobridge-diagnose-minimal";

/// The profile the required-child diagnosis context claims.
const REQUIRED_PROFILE: &str =
    "http://example.org/fhir/StructureDefinition/ferrobridge-diagnose-required";

/// The profile the subject-resolving diagnosis context claims.
const SUBJECT_PROFILE: &str =
    "http://example.org/fhir/StructureDefinition/ferrobridge-diagnose-subject";

/// The context mapping whose chain resolves the subject on its own.
const SUBJECT_CONTEXT: &str = "ferrobridge_diagnose_subject";

/// The template both contexts compile against.
const TEMPLATE: &str = "ferrobridge.diagnose.v1";

/// A synthetic `OBJECT_VERSION_ID` for a composition a CDR would have stamped.
const COMPOSITION_UID: &str = "d2b3c1a0-0000-4000-8000-000000000001::ferrobridge.example::1";

/// The run timestamp the tests pin, so nothing reads a clock.
const NOW: &str = "2026-09-15T10:00:00Z";

/// The `Device` reference a call that supplies no `context.who` gets.
const DEVICE: &str = "Device/ferrobridge";

/// Returns the text of one vendored draft example.
fn draft(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(std::fs::read_to_string(format!(
        "{FIXTURES}/draft-rest-api/{name}"
    ))?)
}

/// Reads JSON text as a `Parameters` resource.
fn parameters(text: &str) -> Result<Parameters, Box<dyn Error>> {
    let parsed: serde_json::Value = serde_json::from_str(text)?;
    let fhir_types::codec::Value::Object(object) =
        fhir_types::codec::Value::from_serde_json(parsed)
    else {
        return Err(Box::<dyn Error>::from("the example is not a JSON object"));
    };
    Ok(Parameters::from_json(
        &object,
        &mut Path::root("Parameters"),
    )?)
}

/// Renders a `Parameters` resource as plain JSON.
fn rendered(parameters: &Parameters) -> Result<serde_json::Value, Box<dyn Error>> {
    Ok(fhir_types::codec::Value::Object(Json::to_json(parameters)?)
        .to_serde_json(&mut Path::root("Parameters"))?)
}

/// Returns `document` with every `composition` string parsed in place.
///
/// The composition travels as an opaque JSON string, so comparing two
/// renderings byte for byte would compare their whitespace and key order too.
/// Parsing the string first is what "modulo whitespace and key order" means.
fn normalized(mut document: serde_json::Value) -> serde_json::Value {
    let Some(list) = document
        .get_mut("parameter")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return document;
    };
    for parameter in list.iter_mut() {
        if parameter.get("name").and_then(serde_json::Value::as_str) != Some("composition") {
            continue;
        }
        let Some(text) = parameter
            .get("valueString")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) else {
            continue;
        };
        if let Some(object) = parameter.as_object_mut() {
            object.insert(String::from("valueString"), parsed);
        }
    }
    document
}

/// Builds the compiled set over the fixtures `contexts` names.
fn program_set(contexts: &[&str]) -> Result<ProgramSet, Box<dyn Error>> {
    let mut set = ProgramSet::new();
    set.insert_template(template()?);
    for context in contexts {
        set.insert_program(compiled(context)?)?;
    }
    Ok(set)
}

/// Returns the settings the tests run under.
///
/// `defaults-for-fields.adoc` puts the composer and the context start time on
/// the engine and the composition language and territory on "the project
/// performing the mapping", so they are settings rather than constants.
fn settings() -> Settings {
    Settings::new(DEVICE, NOW).with_defaults(
        fhirconnect::engine::traverse::Defaults::at(NOW)
            .with_language("en")
            .with_territory("NL"),
    )
}

/// Returns the synthetic `Condition` of the testkit, claiming `profile`.
fn condition(profile: &str) -> Result<serde_json::Value, Box<dyn Error>> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = parsed.as_object_mut() {
        object.insert(
            String::from("meta"),
            serde_json::json!({ "profile": [profile] }),
        );
    }
    Ok(parsed)
}

/// Returns a `collection` Bundle carrying `resources`.
fn bundle(resources: Vec<serde_json::Value>) -> Result<Bundle, Box<dyn Error>> {
    let mut entry = Vec::new();
    for resource in resources {
        let fhir_types::codec::Value::Object(object) =
            fhir_types::codec::Value::from_serde_json(resource)
        else {
            return Err(Box::<dyn Error>::from("a resource is not a JSON object"));
        };
        let name = object
            .get("resourceType")
            .and_then(fhir_types::codec::Value::as_str)
            .unwrap_or("Resource")
            .to_owned();
        entry.push(BundleEntry {
            resource: Some(Resource::from_json(&object, &mut Path::root(&name))?),
            ..BundleEntry::default()
        });
    }
    Ok(Bundle {
        r#type: Code::from("collection"),
        entry,
        ..Bundle::default()
    })
}

/// Returns a `Reference` whose literal reference is `text`.
fn reference(text: &str) -> Reference {
    Reference {
        reference: Some(fhir_types::r4::primitives::String::from(text)),
        ..Reference::default()
    }
}

/// Maps the synthetic `Condition` into a composition and stamps a `uid` on it.
///
/// A composition a CDR served always carries its `OBJECT_VERSION_ID`; the
/// engine's builder stamps none, so the test stamps the one a commit would
/// have written.
fn committed_composition() -> Result<serde_json::Value, Box<dyn Error>> {
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let request = ToOpenehrRequest::new(bundle(vec![condition(SUBJECT_PROFILE)?])?);
    let answer = run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let mut built: serde_json::Value = serde_json::from_str(answer.composition())?;
    if let Some(object) = built.as_object_mut() {
        object.insert(
            String::from("uid"),
            serde_json::json!({ "_type": "OBJECT_VERSION_ID", "value": COMPOSITION_UID }),
        );
    }
    Ok(built)
}
