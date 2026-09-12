// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The whole FHIRconnect mapping library is loaded into one registry here.
//!
//! `metadata.name` is "a unique id used to identify the mapping and reference
//! it" (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`,
//! §Metadata), so the library's repeated names cannot all load together. The
//! test pins which names repeat and how many files each spans.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::loader;
use openehr_mapping_core::registry::MappingRegistry;
use openehr_mapping_core::registry::RegistryError;

/// The names the mapping library repeats, and how many files declare each.
///
/// The six `KDS_composition` files and the two `KDS_Person.context` files are
/// the library's own duplicates; the first file of each set loads and the rest
/// are refused.
const EXPECTED_DUPLICATES: &[(&str, usize)] = &[("KDS_Person.context", 2), ("KDS_composition", 6)];

/// The repository root, reached from this crate's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Returns every `.yml` and `.yaml` file under `directory`, in path order.
fn yaml_files(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    let mut stack = vec![directory.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(std::ffi::OsStr::to_str),
                Some("yml" | "yaml")
            ) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

#[test]
fn the_mapping_library_refuses_its_repeated_names() -> Result<(), Box<dyn Error>> {
    let root = repo_root().canonicalize()?;
    let mut registry = MappingRegistry::new();
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();

    for file in yaml_files(&root.join("docs/specs/fhirconnect-mapping-lib"))? {
        let Ok(document) = loader::load_file(file.clone()) else {
            continue;
        };
        if let Err(error) = registry.insert(document) {
            let RegistryError::DuplicateName {
                ref name,
                ref first_file,
                ref second_file,
                ..
            } = error;
            assert_ne!(first_file, second_file, "a duplicate names two files");
            assert_eq!(second_file, &file, "the refusal names the refused file");

            let diagnostic = Diagnostic::from(&error);
            assert_eq!(diagnostic.code(), &DiagnosticCode::DuplicateMappingName);
            let rendered = diagnostic.to_string();
            assert!(
                rendered.contains(&first_file.display().to_string()),
                "{rendered}"
            );
            assert!(
                rendered.contains(&second_file.display().to_string()),
                "{rendered}"
            );

            *refusals.entry(name.as_str().to_owned()).or_default() += 1;
        }
    }

    let counted: Vec<(String, usize)> = refusals
        .into_iter()
        .map(|(name, refused)| (name, refused + 1))
        .collect();
    let expected: Vec<(String, usize)> = EXPECTED_DUPLICATES
        .iter()
        .map(|(name, files)| ((*name).to_owned(), *files))
        .collect();
    assert_eq!(counted, expected, "the library's duplicate names moved");
    Ok(())
}

#[test]
fn the_omocl_corpus_has_no_repeated_name() -> Result<(), Box<dyn Error>> {
    let root = repo_root().canonicalize()?;
    let mut registry = MappingRegistry::new();
    let mut refused = Vec::new();
    for file in yaml_files(&root.join("docs/specs/omocl"))? {
        let Ok(document) = loader::load_file(file) else {
            continue;
        };
        if let Err(error) = registry.insert(document) {
            refused.push(error.to_string());
        }
    }
    assert_eq!(refused, Vec::<String>::new());
    assert_eq!(registry.len(), 201);
    Ok(())
}
