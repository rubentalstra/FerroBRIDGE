// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Both vendored corpora are loaded here, and the exact set of files this
//! crate refuses is pinned.
//!
//! Neither corpus is an oracle (`.claude/rules/spec-adherence.md`): a file
//! that fails is evidence about the corpus, adjudicated in the list below,
//! never a reason to widen the loader.

use std::error::Error;
use std::path::Path;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::header::MappingLanguage;
use openehr_mapping_core::loader;

/// The two vendored mapping corpora, relative to the repository root.
const CORPORA: &[&str] = &["docs/specs/fhirconnect-mapping-lib", "docs/specs/omocl"];

/// Every corpus file this crate refuses, with the code it refuses it under.
///
/// `lab_composition.yml` is commented out from its first byte to its last, so
/// the YAML document it holds is null and carries no header at all.
/// `KDS_problem_diagnose.yml` writes `"extensionValue"#TODO` with no space
/// before the `#`, and YAML 1.2.2 §6.6 requires that "comments must be
/// separated from other tokens by white space characters".
/// `Menstrual_diary_v1.yml` writes the key `at0005` five times inside one
/// `conceptMap` `mapping` block, so four of its five concept mappings would be
/// discarded silently. The other 306 files load.
const EXPECTED_FAILURES: &[(&str, DiagnosticCode)] = &[
    (
        "docs/specs/fhirconnect-mapping-lib/projects/org.highmed/KDS/diagnose/KDS_problem_diagnose.yml",
        DiagnosticCode::YamlSyntax,
    ),
    (
        "docs/specs/fhirconnect-mapping-lib/projects/org.openehr/EEHRxF/lab/bundle/lab_composition.yml",
        DiagnosticCode::EmptyDocument,
    ),
    (
        "docs/specs/omocl/medical_data/observation/Menstrual_diary_v1.yml",
        DiagnosticCode::YamlDuplicateKey,
    ),
];

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

/// Returns the path of `file` relative to the repository root, in `/` form.
fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn every_corpus_file_loads_or_fails_with_the_pinned_diagnostic() -> Result<(), Box<dyn Error>> {
    let root = repo_root().canonicalize()?;
    let mut loaded = 0_usize;
    let mut failures: Vec<(String, DiagnosticCode)> = Vec::new();

    for corpus in CORPORA {
        for file in yaml_files(&root.join(corpus))? {
            match loader::load_file(file.clone()) {
                Ok(_) => loaded += 1,
                Err(error) => {
                    let diagnostic = Diagnostic::from(&error);
                    assert_eq!(
                        diagnostic.file(),
                        file.as_path(),
                        "a diagnostic names the file it is about"
                    );
                    failures.push((relative(&root, &file), diagnostic.code().clone()));
                }
            }
        }
    }

    failures.sort();
    let expected: Vec<(String, DiagnosticCode)> = EXPECTED_FAILURES
        .iter()
        .map(|(file, code)| ((*file).to_owned(), code.clone()))
        .collect();
    assert_eq!(failures, expected, "the corpus failure set moved");
    assert_eq!(loaded, 306, "the corpus size moved");
    Ok(())
}

#[test]
fn the_two_corpora_declare_the_two_grammars() -> Result<(), Box<dyn Error>> {
    let root = repo_root().canonicalize()?;
    for (corpus, language) in [
        (
            "docs/specs/fhirconnect-mapping-lib",
            MappingLanguage::FhirConnect,
        ),
        ("docs/specs/omocl", MappingLanguage::Omocl),
    ] {
        for file in yaml_files(&root.join(corpus))? {
            if let Ok(document) = loader::load_file(file.clone()) {
                assert_eq!(
                    document.header().grammar().value().language(),
                    language,
                    "{} declares the wrong grammar",
                    relative(&root, &file)
                );
            }
        }
    }
    Ok(())
}

#[test]
fn every_loaded_omocl_file_names_an_archetype() -> Result<(), Box<dyn Error>> {
    let root = repo_root().canonicalize()?;
    let mut counted = 0_usize;
    for file in yaml_files(&root.join("docs/specs/omocl"))? {
        let Ok(document) = loader::load_file(file.clone()) else {
            continue;
        };
        assert!(
            document.header().archetype().is_some(),
            "{} names no archetype",
            relative(&root, &file)
        );
        counted += 1;
    }
    assert_eq!(counted, 201, "the OMOCL corpus size moved");
    Ok(())
}
