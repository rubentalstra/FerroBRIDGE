// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the suites share: the vendored corpus walk and the diagnostic
//! renderer.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;

/// The vendored OMOCL corpus, relative to this crate's manifest.
pub(crate) const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/specs/omocl");

/// Returns every mapping file of the corpus, by path relative to its root.
pub(crate) fn corpus_files() -> Result<Vec<String>, Box<dyn Error>> {
    let root = PathBuf::from(CORPUS);
    let mut found = Vec::new();
    collect(&root, &root, &mut found)?;
    found.sort();
    Ok(found)
}

/// Returns the absolute path of a corpus file.
pub(crate) fn corpus_path(relative: &str) -> PathBuf {
    PathBuf::from(CORPUS).join(relative)
}

/// Walks `directory`, collecting every YAML file relative to `root`.
fn collect(root: &Path, directory: &Path, found: &mut Vec<String>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, found)?;
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "yml" || extension == "yaml");
        if is_yaml {
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            found.push(relative);
        }
    }
    Ok(())
}

/// Renders a diagnostic list as one error message.
pub(crate) fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join("\n")
}
