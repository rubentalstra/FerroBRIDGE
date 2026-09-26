// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Running one program, and reading the identity of the entry it maps.
//!
//! The facade is a client of the same in-process engine the `$tofhir` and
//! `$toopenehr` operations call, so the two surfaces cannot disagree (no
//! specification governs this: our own design). This module is the thin seam: it hands the
//! engine the element table and the template index, and it reads back out of
//! the produced composition the two facts identity needs, the entry's path and
//! its `LOCATABLE.uid`.

use fhir_types::codec::Value;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::origin::Origin;
use fhirconnect::engine::origin::SourceItem;
use fhirconnect::engine::outcome::Outcome;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::Defaults;
use fhirconnect::engine::traverse::error::EngineError;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::engine::traverse::to_openehr;
use fhirconnect::resolve::program::Program;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;

/// The composer the facade records when no mapping sets one.
///
/// "The composer … could be defaulted with a value such as `FHIRconnect`
/// party" (`engine/defaults-for-fields.adoc`); this deployment names itself so
/// a reader can tell which bridge wrote the composition.
pub const COMPOSER: &str = "FerroBRIDGE";

/// Maps one FHIR document into a composition.
///
/// `now` is the one instant this ingest defaults every composition field
/// from, so a single request cannot carry two clock readings. The composition
/// carries a `FEEDER_AUDIT` naming this bridge as the system, the document's
/// `resourceType`, `id` and `meta.versionId` as the source item, and every
/// field the engine defaulted, because `FEEDER_AUDIT` "describes the origin of
/// data that have been transformed into openEHR form and committed to the
/// system" (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html>).
///
/// # Errors
///
/// Returns [`EngineError`] for any element the program cannot map, and for a
/// set of values the template does not admit as a composition.
pub fn inbound(
    program: &Program,
    index: &WebTemplateIndex,
    document: &Value,
    now: &str,
    settings: &crate::facade::Settings,
) -> Result<Outcome<CanonicalComposition>, EngineError> {
    inbound_from(
        program,
        index,
        document,
        now,
        settings,
        SourceItem::of(document),
    )
}

/// Maps one FHIR document into a composition that names `source` as its
/// origin.
///
/// This is [`inbound`] with the originating item given by the caller, so a
/// face that received something other than the resource itself, such as a
/// message, names what it received. `None` records no source item.
///
/// # Errors
///
/// Returns [`EngineError`] for any element the program cannot map, and for a
/// set of values the template does not admit as a composition.
pub fn inbound_from(
    program: &Program,
    index: &WebTemplateIndex,
    document: &Value,
    now: &str,
    settings: &crate::facade::Settings,
    source: Option<SourceItem>,
) -> Result<Outcome<CanonicalComposition>, EngineError> {
    // TODO(#95): run the terminology calls the concept mappings need here,
    // before any value is built, once the PROGRAMMED registry lands.
    let mut origin = Origin::new(&settings.system_id);
    if let Some(source) = source {
        origin = origin.with_source(source);
    }
    let defaults = Defaults::at(now)
        .with_composer(COMPOSER)
        .with_language(&settings.language)
        .with_territory(&settings.territory)
        .with_origin(origin);
    to_openehr(
        program,
        &fhir_types::r4::schema::SCHEMAS,
        index,
        document,
        &Seams::default(),
        &defaults,
        &CallContext::new(),
    )
}

/// Maps one composition into a FHIR document.
///
/// # Errors
///
/// Returns [`EngineError`] for any element the program cannot map.
pub fn outbound(
    program: &Program,
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
) -> Result<Outcome<Value>, EngineError> {
    to_fhir(
        program,
        &fhir_types::r4::schema::SCHEMAS,
        index,
        composition,
        &Seams::default(),
        &CallContext::new(),
    )
}

/// Where in a composition the entry one program maps sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The archetype path of the entry, as an openEHR path writes it.
    path: String,
    /// The `LOCATABLE.uid` the entry carries, when it carries one.
    uid: Option<String>,
}

impl Entry {
    /// Returns the archetype path of the entry.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the `LOCATABLE.uid` the entry carries.
    #[must_use]
    pub fn uid(&self) -> Option<&str> {
        self.uid.as_deref()
    }
}

/// Returns the entry the start model mapping of `program` writes.
///
/// A context maps one profile onto one template through a start model mapping
/// whose `metadata.archetype` names the entry
/// (`types-of-mapping-files/context-mappings.adoc`), so the entry is the
/// `content` item carrying that `archetype_node_id`. The path is the openEHR
/// one (<https://specifications.openehr.org/releases/BASE/Release-1.2.0/architecture_overview.html>),
/// positionally indexed when the composition holds several items of the
/// archetype.
///
/// A composition with no such item answers `None`: the program mapped no entry
/// of its own archetype, which the caller reports rather than papering over.
#[must_use]
pub fn entry_of(program: &Program, composition: &CanonicalComposition) -> Option<Entry> {
    entry_at(composition, &archetype_of(program)?)
}

/// Returns the `content` item of `composition` carrying `archetype`.
fn entry_at(composition: &CanonicalComposition, archetype: &str) -> Option<Entry> {
    let content = composition.value().get("content")?.as_array()?;
    let matching: Vec<(usize, &serde_json::Value)> = content
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.get("archetype_node_id")
                .and_then(serde_json::Value::as_str)
                == Some(archetype)
        })
        .collect();
    let (index, item) = matching.first().copied()?;
    let path = if matching.len() > 1 {
        // NOTE: openEHR BASE Release 1.2.0 §Paths and Locators writes the
        // positional predicate 1-based, so the `content` index converts.
        format!("/content[{archetype}, {}]", index.saturating_add(1))
    } else {
        format!("/content[{archetype}]")
    };
    Some(Entry {
        path,
        uid: item
            .get("uid")
            .and_then(|uid| uid.get("value"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

/// Returns the archetype the start model mapping of `program` declares.
fn archetype_of(program: &Program) -> Option<String> {
    program
        .models()
        .iter()
        .find(|model| model.name() == program.start())
        .and_then(|model| model.archetype())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::entry_at;
    use openehr_mapping_core::composition::CanonicalComposition;
    use openehr_mapping_core::template::Generation;

    /// The archetype every case below looks for.
    const DIAGNOSIS: &str = "openEHR-EHR-EVALUATION.problem_diagnosis.v1";

    /// Returns a composition carrying `items` under `content`.
    fn composition(items: &serde_json::Value) -> CanonicalComposition {
        CanonicalComposition::new(
            serde_json::json!({ "_type": "COMPOSITION", "content": items }),
            "ferrobridge.diagnose.v1",
            Generation::Adl14,
        )
    }

    #[test]
    fn one_entry_of_an_archetype_has_an_unindexed_path() {
        let built = composition(&serde_json::json!([{ "archetype_node_id": DIAGNOSIS }]));
        let entry = entry_at(&built, DIAGNOSIS).expect("the entry is found");
        assert_eq!(format!("/content[{DIAGNOSIS}]"), entry.path());
        assert_eq!(None, entry.uid());
    }

    #[test]
    fn several_entries_of_one_archetype_take_a_one_based_predicate() {
        let built = composition(&serde_json::json!([
            { "archetype_node_id": DIAGNOSIS },
            { "archetype_node_id": DIAGNOSIS }
        ]));
        let entry = entry_at(&built, DIAGNOSIS).expect("the entry is found");
        assert_eq!(format!("/content[{DIAGNOSIS}, 1]"), entry.path());
    }

    #[test]
    fn an_entry_uid_is_read_when_the_composition_carries_one() {
        let built = composition(&serde_json::json!([{
            "archetype_node_id": DIAGNOSIS,
            "uid": { "_type": "HIER_OBJECT_ID", "value": "3a2f1c4e-0000-4000-8000-0000000000ab" }
        }]));
        let entry = entry_at(&built, DIAGNOSIS).expect("the entry is found");
        assert_eq!(Some("3a2f1c4e-0000-4000-8000-0000000000ab"), entry.uid());
    }

    #[test]
    fn a_composition_holding_no_such_entry_answers_none() {
        let built = composition(&serde_json::json!([
            { "archetype_node_id": "openEHR-EHR-OBSERVATION.body_weight.v2" }
        ]));
        assert!(entry_at(&built, DIAGNOSIS).is_none());
        assert!(entry_at(&composition(&serde_json::json!([])), DIAGNOSIS).is_none());
    }
}
