// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The comparison of a mapped Bundle with the one its set expects, each
//! difference classified against what the guide's maps target.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use ferrobridge_hl7v2::map::corpus::{Condition, Kind};
use fhir_types::codec::Value;

use crate::support;

/// What the guide's maps target, to classify a difference from an expected
/// Bundle.
pub(super) struct Targets {
    /// Per message map, by its structure name: each resource type its rows
    /// write, with whether every such row is narrative-gated.
    resources: BTreeMap<String, BTreeMap<String, bool>>,
    /// Per resource type and top-level element: whether every segment map row
    /// that writes it is narrative-gated.
    elements: BTreeMap<(String, String), bool>,
}

/// The leading name of a target such as `Patient[1]` or `identifier[2].value`.
fn head(target: &str) -> &str {
    target.split(['[', '.', '(']).next().unwrap_or(target)
}

impl Targets {
    pub(super) fn of(guide: &ferrobridge_hl7v2::map::corpus::Corpus) -> Self {
        let mut resources: BTreeMap<String, BTreeMap<String, bool>> = BTreeMap::new();
        let mut elements: BTreeMap<(String, String), bool> = BTreeMap::new();
        for concept_map in guide.maps() {
            match concept_map.kind {
                Kind::Message => {
                    let Some(structure) = concept_map
                        .rows
                        .first()
                        .and_then(|row| row.source.split(['.', ':']).next())
                    else {
                        continue;
                    };
                    let written = resources.entry(structure.to_owned()).or_default();
                    for row in &concept_map.rows {
                        let narrative = row.condition == Condition::Narrative;
                        let every = written
                            .entry(head(&row.target_code).to_owned())
                            .or_insert(true);
                        *every = *every && narrative;
                    }
                }
                Kind::Segment => {
                    let Some((_, resource)) = concept_map.id.rsplit_once("-to-") else {
                        continue;
                    };
                    for row in &concept_map.rows {
                        let narrative = row.condition == Condition::Narrative;
                        let key = (
                            resource.to_ascii_lowercase(),
                            head(&row.target_code).to_owned(),
                        );
                        let every = elements.entry(key).or_insert(true);
                        *every = *every && narrative;
                    }
                }
                Kind::Datatype | Kind::Table | Kind::Other => {}
            }
        }
        Self {
            resources,
            elements,
        }
    }

    /// The class of a resource type the expected Bundle holds and the run
    /// does not.
    fn resource(&self, structure: &str, resource: &str) -> &'static str {
        match self
            .resources
            .get(structure)
            .and_then(|written| written.get(resource))
        {
            None => "corpus-gap",
            Some(true) => "supplement",
            Some(false) => "defect",
        }
    }

    /// The class of a top-level element the expected Bundle writes on a
    /// resource type and the run does not.
    fn element(&self, resource: &str, element: &str) -> &'static str {
        match self
            .elements
            .get(&(resource.to_ascii_lowercase(), element.to_owned()))
        {
            None => "corpus-gap",
            Some(true) => "supplement",
            Some(false) => "defect",
        }
    }
}

/// The resources of a Bundle by type: how many, and the union of their
/// top-level element names.
fn inventory(bundle: &Value) -> BTreeMap<String, (usize, BTreeSet<String>)> {
    let mut found: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    let entries = bundle
        .get("entry")
        .and_then(Value::as_array)
        .unwrap_or_default();
    for resource in entries.iter().filter_map(|entry| entry.get("resource")) {
        let (Some(kind), Some(object)) = (
            resource.get("resourceType").and_then(Value::as_str),
            resource.as_object(),
        ) else {
            continue;
        };
        let slot = found.entry(kind.to_owned()).or_default();
        slot.0 = slot.0.saturating_add(1);
        slot.1.extend(
            object
                .keys()
                .filter(|key| !matches!(key.as_str(), "resourceType" | "id" | "meta"))
                .cloned(),
        );
    }
    found
}

/// Reads an expected Bundle, JSON or XML by its extension.
pub(super) fn expected_bundle(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    if path.extension().is_some_and(|extension| extension == "xml") {
        fhir_types::xml::from_xml(&fhir_types::r4::schema::SCHEMAS, text)
            .map(Value::Object)
            .map_err(|error| error.to_string())
    } else {
        serde_json::from_str(text).map_err(|error| error.to_string())
    }
}

/// Counts every difference between the run's Bundle and the expected one,
/// resource type by resource type and element by element, each classified:
/// `corpus-gap` where no map of the guide writes it, `supplement` (#256)
/// where every row that writes it is narrative-gated, `defect` where a row
/// writes it and the run did not, and `beyond-oracle` where the run writes
/// what the expected Bundle leaves out.
pub(super) fn compare(
    ours: &Value,
    expected: &Value,
    structure: &str,
    targets: &Targets,
    outcomes: &mut BTreeMap<String, usize>,
) {
    let ours = inventory(ours);
    let theirs = inventory(expected);
    let mut count = |key: String| {
        let slot = outcomes.entry(key).or_insert(0);
        *slot = slot.saturating_add(1);
    };
    for (kind, (expected_count, expected_elements)) in &theirs {
        let Some((our_count, our_elements)) = ours.get(kind) else {
            count(format!(
                "compare:{}:resource:{kind}",
                targets.resource(structure, kind)
            ));
            continue;
        };
        if our_count != expected_count {
            count(format!("compare:count:{kind}"));
        }
        for element in expected_elements.difference(our_elements) {
            count(format!(
                "compare:{}:element:{kind}.{element}",
                targets.element(kind, element)
            ));
        }
        for element in our_elements.difference(expected_elements) {
            count(format!("compare:beyond-oracle:element:{kind}.{element}"));
        }
    }
    for kind in ours.keys().filter(|kind| !theirs.contains_key(*kind)) {
        count(format!("compare:beyond-oracle:resource:{kind}"));
    }
}

#[test]
fn a_difference_the_guide_maps_is_a_defect_and_one_it_never_maps_a_corpus_gap() {
    let guide = support::corpus();
    let targets = Targets::of(&guide);
    assert_eq!(targets.resource("ORU_R01", "Patient"), "defect");
    assert_eq!(targets.resource("ORU_R01", "Immunization"), "corpus-gap");
    assert_eq!(targets.element("Patient", "name"), "defect");
    assert_eq!(targets.element("Patient", "photo"), "corpus-gap");
}
