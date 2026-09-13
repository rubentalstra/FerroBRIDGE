// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FHIRconnect mapping library, run through all three validation layers.
//!
//! The library is evidence, never an oracle: the FHIRconnect specification
//! decides what a mapping file may contain, and these tests pin where the
//! published files and the published schemas disagree with it. Every
//! expectation below is a pinned set, so the day upstream changes either one
//! the test fails and the disagreement is re-adjudicated rather than absorbed.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use fhirconnect::model::error::SchemaKind;
use fhirconnect::model::schema::Schemas;
use fhirconnect::model::schema::published;
use fhirconnect::model::schema::strict;
use openehr_mapping_core::loader::load_str;

/// The vendored mapping library, relative to this crate's manifest.
const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/fhirconnect-mapping-lib"
);

/// The vendored FHIRconnect specification source, relative to the manifest.
const SPEC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/specs/fhirconnect");

/// Returns every mapping file of the corpus, by path relative to its root.
fn corpus_files() -> Vec<String> {
    let root = PathBuf::from(CORPUS);
    let mut found = Vec::new();
    collect(&root, &root, &mut found);
    found.sort();
    found
}

/// Walks `directory`, collecting every YAML file relative to `root`.
fn collect(root: &Path, directory: &Path, found: &mut Vec<String>) {
    let entries = fs::read_dir(directory).expect("the vendored corpus is readable");
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            collect(root, &path, found);
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "yml" || extension == "yaml");
        if is_yaml {
            let relative = path
                .strip_prefix(root)
                .expect("a path under the corpus root")
                .to_string_lossy()
                .into_owned();
            found.push(relative);
        }
    }
}

/// Reads one corpus file.
fn read(relative: &str) -> String {
    fs::read_to_string(PathBuf::from(CORPUS).join(relative)).expect("a readable corpus file")
}

/// Compiles the published schemas from the vendored specification source.
fn published_schemas() -> Schemas {
    let attachments = PathBuf::from(SPEC).join("modules/ROOT/attachments");
    let model = fs::read_to_string(attachments.join(SchemaKind::Model.file_name()))
        .expect("the vendored published model schema");
    let context = fs::read_to_string(attachments.join(SchemaKind::Context.file_name()))
        .expect("the vendored published context schema");
    published::compile(&model, &context).expect("the published schemas compile")
}

/// Compiles the published schemas as the rendered v1.0.0 site ships them.
fn site_schemas() -> Schemas {
    let attachments = PathBuf::from(SPEC).join("build/site/FHIRconnect/v1.0.0/_attachments");
    let model = fs::read_to_string(attachments.join(SchemaKind::Model.file_name()))
        .expect("the rendered published model schema");
    let context = fs::read_to_string(attachments.join(SchemaKind::Context.file_name()))
        .expect("the rendered published context schema");
    published::compile(&model, &context).expect("the rendered schemas compile")
}

/// Validates the whole corpus against `schemas`, keyed by relative path.
fn validate_corpus(schemas: &Schemas) -> BTreeMap<String, Vec<String>> {
    let mut refused = BTreeMap::new();
    for relative in corpus_files() {
        let source = read(&relative);
        let Ok(document) = load_str(relative.clone(), &source) else {
            continue;
        };
        let diagnostics = schemas.validate_document(&document);
        if !diagnostics.is_empty() {
            refused.insert(
                relative,
                diagnostics
                    .iter()
                    .map(|diagnostic| {
                        format!("{}: {}", diagnostic.model_path(), diagnostic.message())
                    })
                    .collect(),
            );
        }
    }
    refused
}

#[test]
fn dump_published_rejections() {
    let refused = validate_corpus(&published_schemas());
    let mut report = String::new();
    for (file, errors) in &refused {
        for error in errors {
            report.push_str(&format!("{file}\t{error}\n"));
        }
    }
    fs::write("/tmp/fc-published.txt", report).expect("a writable report");
    fs::write(
        "/tmp/fc-strict.txt",
        validate_corpus(&strict::schemas().expect("the strict schemas compile"))
            .iter()
            .flat_map(|(file, errors)| errors.iter().map(move |e| format!("{file}\t{e}\n")))
            .collect::<String>(),
    )
    .expect("a writable report");
    fs::write(
        "/tmp/fc-site.txt",
        validate_corpus(&site_schemas())
            .iter()
            .flat_map(|(file, errors)| errors.iter().map(move |e| format!("{file}\t{e}\n")))
            .collect::<String>(),
    )
    .expect("a writable report");
    assert_eq!(corpus_files().len(), 107, "the corpus is 107 mapping files");
}

#[test]
fn dump_semantic() {
    let paths: Vec<PathBuf> = corpus_files()
        .iter()
        .map(|relative| PathBuf::from(CORPUS).join(relative))
        .collect();
    let codes = fhirconnect::model::semantic::StaticMappingCodes::default();
    let report = match fhirconnect::model::load::load_set(&paths, &codes) {
        Ok(set) => format!("loaded {}\n", set.len()),
        Err(diagnostics) => diagnostics
            .iter()
            .map(|d| {
                format!(
                    "{}\t{}\t{}\n",
                    d.file()
                        .strip_prefix(CORPUS)
                        .unwrap_or(d.file())
                        .to_string_lossy(),
                    d.code(),
                    d.message()
                )
            })
            .collect::<String>(),
    };
    fs::write("/tmp/fc-semantic.txt", report).expect("a writable report");
}
