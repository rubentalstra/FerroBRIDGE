// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The supplements this crate ships: `ConceptMaps` in the guide's own shape
//! that override a map of `hl7.fhir.uv.v2mappings` 1.0.0 or add one it lacks.
//!
//! Each file sits under the crate's `supplements/` directory and is compiled
//! in, so the published crate carries them without the vendored package.
//! Each names FerroBRIDGE in its `title`, and its `description` names the
//! guide map it overrides or the structure it adds, the rows it changes and
//! why, with the tracker issue that records the defect. An override keeps the
//! guide map's id and canonical url; an added map takes an id of the guide's
//! naming form and a url under FerroBRIDGE's own base, so no added map claims
//! a canonical of the guide. [`Corpus::with_shipped_supplements`] loads them,
//! and the run counts each one it uses as `supplemented`.
//!
//! No specification governs this: our own design, under the guide's leave to
//! add a mapping an implementation needs locally (`mapping_guidelines.md`
//! §General Format/Approach).
//!
//! [`Corpus::with_shipped_supplements`]: crate::map::corpus::Corpus::with_shipped_supplements

/// The directory the files are read from, relative to the crate root, which
/// names them in a load error.
pub const DIRECTORY: &str = "supplements";

/// The canonical base of a map a supplement adds.
pub const CANONICAL_BASE: &str = "https://ferrobridge.eu/fhir/v2mappings/ConceptMap/";

/// One entry of [`SHIPPED`]: the file name and the file's text.
macro_rules! shipped {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/supplements/", $name)),
        )
    };
}

/// Every shipped supplement, as its file name and its text, in file name
/// order.
pub const SHIPPED: [(&str, &str); 17] = [
    shipped!("ConceptMap-datatype-ce-to-codeableconcept.json"),
    shipped!("ConceptMap-datatype-cf-to-codeableconcept.json"),
    shipped!("ConceptMap-datatype-cwe-to-codeableconcept.json"),
    shipped!("ConceptMap-datatype-cwe-to-quantity.json"),
    shipped!("ConceptMap-datatype-hd-name-to-messageheader-destination.json"),
    shipped!("ConceptMap-datatype-hd-name-to-messageheader-source.json"),
    shipped!("ConceptMap-datatype-pl-to-location.json"),
    shipped!("ConceptMap-message-adt-a03-to-bundle.json"),
    shipped!("ConceptMap-message-adt-a05-to-bundle.json"),
    shipped!("ConceptMap-message-adt-a09-to-bundle.json"),
    shipped!("ConceptMap-message-bar-p01-to-bundle.json"),
    shipped!("ConceptMap-message-orl-o22-to-bundle.json"),
    shipped!("ConceptMap-message-oul-r22-to-bundle.json"),
    shipped!("ConceptMap-segment-msh-to-messageheader.json"),
    shipped!("ConceptMap-segment-orc-to-diagnosticreport.json"),
    shipped!("ConceptMap-segment-pid-to-appointment.json"),
    shipped!("ConceptMap-segment-sch-to-appointment.json"),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use fhir_types::codec::{Json, Value};
    use fhir_types::r4::concept_map::ConceptMap;

    use super::{CANONICAL_BASE, DIRECTORY, SHIPPED};
    use crate::map::corpus::{CANONICAL_BASE as GUIDE_BASE, Corpus, Origin};

    fn package() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/package")
    }

    fn decoded(text: &str) -> ConceptMap {
        let value: Value = serde_json::from_str(text).expect("a JSON document");
        let object = value.as_object().expect("a JSON object");
        ConceptMap::from_json(object, &mut fhir_types::codec::Path::root("ConceptMap"))
            .expect("an R4 ConceptMap")
    }

    #[test]
    fn every_file_of_the_directory_is_shipped_in_name_order() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(DIRECTORY);
        let on_disk: Vec<String> = std::fs::read_dir(&directory)
            .expect("the supplements directory reads")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let shipped: Vec<String> = SHIPPED.iter().map(|(name, _)| (*name).to_owned()).collect();
        assert_eq!(shipped, on_disk);
    }

    // NOTE: no specification governs this: our own design; a supplement names FerroBRIDGE,
    // says what it changes and why, and never claims a canonical of the guide it adds to.
    #[test]
    fn every_supplement_names_ferrobridge_and_the_guide_map_it_changes() {
        let guide = Corpus::load(&package()).expect("the vendored package loads");
        for (name, text) in SHIPPED {
            let map = decoded(text);
            let id = map.id.clone().expect("an id");
            assert_eq!(name, format!("ConceptMap-{id}.json"), "{name}");
            let title = map.title.and_then(|title| title.value).unwrap_or_default();
            assert!(
                title.starts_with("FerroBRIDGE supplement: "),
                "{name}: {title}"
            );
            let description = map
                .description
                .and_then(|description| description.value)
                .unwrap_or_default();
            assert!(
                description.contains("FerroBRIDGE #"),
                "{name}: {description}"
            );
            let url = map.url.and_then(|url| url.value).unwrap_or_default();
            if let Some(replaced) = guide.get(&id) {
                assert_eq!(url, replaced.url, "{name} keeps the guide map's url");
                assert!(
                    description.starts_with(&format!(
                        "Overrides ConceptMap/{id} of hl7.fhir.uv.v2mappings 1.0.0."
                    )),
                    "{name}: {description}"
                );
            } else {
                assert_eq!(url, format!("{CANONICAL_BASE}{id}"), "{name}");
                assert!(!url.starts_with(GUIDE_BASE), "{name}");
                assert!(description.starts_with("Adds "), "{name}: {description}");
            }
        }
    }

    #[test]
    fn the_shipped_supplements_load_over_the_guide_as_overrides_and_additions() {
        let guide = Corpus::load(&package()).expect("the vendored package loads");
        let count = guide.maps().count();
        let supplemented = guide
            .clone()
            .with_shipped_supplements()
            .expect("the shipped supplements load");
        let mut added = 0usize;
        for (_, text) in SHIPPED {
            let id = decoded(text).id.expect("an id");
            let map = supplemented.get(&id).expect("the supplement is loaded");
            let overrides = guide.get(&id).is_some();
            assert_eq!(map.origin, Origin::Supplement { overrides }, "{id}");
            if !overrides {
                added = added.saturating_add(1);
            }
        }
        assert_eq!(supplemented.maps().count(), count.saturating_add(added));
        assert!(
            supplemented
                .maps()
                .filter(|map| guide.get(&map.id).is_some())
                .filter(|map| {
                    let file = format!("ConceptMap-{}.json", map.id);
                    SHIPPED.iter().all(|(name, _)| *name != file)
                })
                .all(|map| map.origin == Origin::Guide)
        );
    }
}
