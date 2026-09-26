// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The interpreter over a compiled program.
//!
//! The chain under test is a FerroBRIDGE-authored model mapping carrying one
//! condition per operator of
//! `docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
//! compiled against the synthetic diagnosis template of the testkit and
//! evaluated over the synthetic FHIR `Condition` of the testkit.

mod audit;
mod chain;
mod conditions;
mod functions;
mod links;
mod reference;
mod split;
mod tail;

use core::cell::RefCell;
use core::error::Error;
use std::collections::BTreeMap;
use std::sync::Arc;

use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::condition::Verdict;
use fhirconnect::engine::condition::evaluate;
use fhirconnect::engine::seam::IdentityRequest;
use fhirconnect::engine::seam::IdentitySink;
use fhirconnect::engine::seam::ReferenceError;
use fhirconnect::engine::seam::ReferenceSource;
use fhirconnect::engine::traverse::Defaults;

use fhirconnect::resolve::program::Program;
use fhirconnect::resolve::program::binding::ResourceType;
use fhirconnect::resolve::program::mapping::Mapping;
use fhirconnect::resolve::program::mapping::MappingParts;
use fhirconnect::resolve::program::mapping::Method;
use fhirconnect::resolve::program::target::Attachment;
use openehr_mapping_core::header::metadata::MappingName;

use crate::support::compiled;

/// Compiles the condition fixture into its program.
fn program() -> Result<Arc<Program>, Box<dyn Error>> {
    compiled("ferrobridge_conditions")
}

/// Returns the compiled mapping of `name`, at any depth.
fn mapping<'program>(program: &'program Program, name: &str) -> Option<&'program Mapping> {
    fn walk<'mapping>(mappings: &'mapping [Mapping], name: &str) -> Option<&'mapping Mapping> {
        for mapping in mappings {
            if mapping.name() == name {
                return Some(mapping);
            }
            if let Some(found) = walk(mapping.followed_by(), name) {
                return Some(found);
            }
            let nested = match *mapping.method() {
                Method::Slot { ref mappings, .. } | Method::Reference { ref mappings, .. } => {
                    mappings.as_slice()
                }
                Method::Value
                | Method::Link { .. }
                | Method::Programmed { .. }
                | Method::Participation { .. } => &[],
            };
            if let Some(found) = walk(nested, name) {
                return Some(found);
            }
        }
        None
    }
    walk(program.mappings(), name)
}

/// Returns the synthetic FHIR `Condition` of the testkit as a lexical tree.
fn condition_document() -> Result<Value, Box<dyn Error>> {
    let parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    Ok(Value::from_serde_json(parsed))
}

/// Evaluates the `fhirCondition` of the mapping named `name`.
fn verdict(name: &str) -> Result<Verdict, Box<dyn Error>> {
    let program = program()?;
    let mapping = mapping(&program, name).ok_or_else(|| format!("no mapping named {name}"))?;
    let condition = mapping
        .fhir_condition()
        .ok_or_else(|| format!("{name} carries no fhirCondition"))?;
    let document = condition_document()?;
    Ok(evaluate(
        &SCHEMAS,
        &document,
        condition,
        condition.attachment() == Attachment::Element,
    )?)
}

/// The timestamp the defaults fill the context start time from.
const NOW: &str = "2026-09-13T08:00:00Z";

/// Returns the defaults a run into openEHR fills its composition fields with.
fn defaults() -> Defaults {
    Defaults::at(NOW).with_language("en").with_territory("NL")
}

/// Returns the synthetic FHIR `Condition` with `onset` set to `onset`.
fn condition_with_onset(onset: &str) -> Result<Value, Box<dyn Error>> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = parsed.as_object_mut() {
        object.insert(
            String::from("onsetDateTime"),
            serde_json::Value::String(String::from(onset)),
        );
    }
    Ok(Value::from_serde_json(parsed))
}

/// Returns the parts of a mapping that hands over to `model`.
fn slot_parts(model: &MappingName, mappings: Vec<Mapping>) -> MappingParts {
    MappingParts {
        name: String::from("slot"),
        model: model.clone(),
        fhir: None,
        openehr: None,
        data_type: None,
        derived: None,
        value: None,
        direction: None,
        fhir_condition: None,
        openehr_condition: None,
        manual: Vec::new(),
        conceptmap: None,
        method: Method::Slot {
            model: model.clone(),
            preprocessors: Vec::new(),
            mappings,
        },
        followed_by: Vec::new(),
    }
}

/// A reference source over a map, keyed by the literal reference.
#[derive(Debug, Default)]
struct MapReferences(BTreeMap<String, Value>);

impl ReferenceSource for MapReferences {
    fn fetch(
        &self,
        reference: &str,
        _expected: &ResourceType,
    ) -> Result<Option<Value>, ReferenceError> {
        Ok(self.0.get(reference).cloned())
    }
}

/// An identity sink over a map from resource type to id, recording every
/// request it answers.
#[derive(Debug, Default)]
struct MapIdentities {
    ids: BTreeMap<String, String>,
    seen: RefCell<Vec<IdentityRequest>>,
}

impl IdentitySink for MapIdentities {
    fn identify(&self, request: &IdentityRequest) -> Result<String, ReferenceError> {
        self.seen.borrow_mut().push(request.clone());
        self.ids
            .get(request.resource_type())
            .cloned()
            .ok_or_else(|| ReferenceError::Identity {
                resource_type: String::from(request.resource_type()),
                source: Box::from("the map holds no id for the type"),
            })
    }
}
