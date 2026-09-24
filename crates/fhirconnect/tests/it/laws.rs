// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two lens laws a round trip asserts, over any compiled program.
//!
//! `PutGet` maps a FHIR resource into openEHR and back; `GetPut` maps a
//! composition into FHIR and back. Neither is an identity: a program declares
//! what it skips and what it defaults, and an element no mapping reaches does
//! not survive. So each law answers the declared set of its run
//! ([`ferrobridge_testkit::laws::declared`]), and the suite pins that set as a
//! reviewed snapshot. The law holds when the output equals the input modulo
//! exactly that set. No specification governs this: our own design.

use core::error::Error;

use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::Defaults;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::engine::traverse::to_openehr;
use fhirconnect::resolve::program::Program;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;

use ferrobridge_testkit::laws::declared;

/// The instant every defaulted time of a law's runs takes.
pub(crate) const NOW: &str = "2026-09-13T08:00:00Z";

/// Returns the defaults a run into openEHR fills its composition fields with.
pub(crate) fn defaults() -> Defaults {
    Defaults::at(NOW).with_language("de").with_territory("DE")
}

/// Changes one value an intermediate document carries, for the corrupted cases.
pub(crate) type Corruption<'a> = &'a dyn Fn(&mut serde_json::Value) -> bool;

/// Leaves the intermediate document as the first leg produced it.
pub(crate) fn untouched(_: &mut serde_json::Value) -> bool {
    true
}

/// Runs `PutGet` over `resource` and returns its declared set.
///
/// `corrupt` edits the composition between the two legs; it answers whether
/// it found what it changes, so a corruption that misses is a failure rather
/// than a clean run.
pub(crate) fn putget(
    program: &Program,
    index: &WebTemplateIndex,
    resource: &serde_json::Value,
    corrupt: Corruption<'_>,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let document = Value::from_serde_json(resource.clone());
    let inbound = to_openehr(
        program,
        &SCHEMAS,
        index,
        &document,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let mut middle = inbound.value().value().clone();
    if !corrupt(&mut middle) {
        return Err("the corruption found nothing to change".into());
    }
    let middle = index.accept(middle)?;
    let outbound = to_fhir(
        program,
        &SCHEMAS,
        index,
        &middle,
        &Seams::default(),
        &CallContext::new(),
    )?;
    let output = outbound
        .value()
        .to_serde_json(&mut fhir_types::codec::Path::root(
            program.resource().as_str(),
        ))?;
    Ok(declared(
        &messages(inbound.warnings()),
        &messages(outbound.warnings()),
        resource,
        &output,
    ))
}

/// Runs `GetPut` over `composition` and returns its declared set.
///
/// `corrupt` edits the FHIR resource between the two legs, as for
/// [`putget`]. The compositions are compared in their FLAT form, whose keys
/// are the template paths (Simplified Formats, §Flat format).
pub(crate) fn getput(
    program: &Program,
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    corrupt: Corruption<'_>,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let outbound = to_fhir(
        program,
        &SCHEMAS,
        index,
        composition,
        &Seams::default(),
        &CallContext::new(),
    )?;
    let mut middle = outbound
        .value()
        .to_serde_json(&mut fhir_types::codec::Path::root(
            program.resource().as_str(),
        ))?;
    if !corrupt(&mut middle) {
        return Err("the corruption found nothing to change".into());
    }
    let inbound = to_openehr(
        program,
        &SCHEMAS,
        index,
        &Value::from_serde_json(middle),
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let input = serde_json::Value::Object(index.flatten(composition)?);
    let output = serde_json::Value::Object(index.flatten(inbound.value())?);
    Ok(declared(
        &messages(outbound.warnings()),
        &messages(inbound.warnings()),
        &input,
        &output,
    ))
}

/// Returns the losses one leg declared, as the messages a reader reviews.
fn messages(warnings: &[fhirconnect::engine::outcome::Warning]) -> Vec<String> {
    warnings.iter().map(ToString::to_string).collect()
}
