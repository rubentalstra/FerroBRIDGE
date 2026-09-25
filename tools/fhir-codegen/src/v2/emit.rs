// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The v2 half of `emit`: the fetched v2 definitions in, `hl7v2-types` out.
//!
//! Every file under the crate's `src/` is generated, so a replace removes the
//! whole tree first and a check reports any file the emitter no longer
//! produces. The files go through the same scratch directory and `rustfmt`
//! pass as `fhir-types`.

use std::fs;
use std::path::PathBuf;

use crate::emit::{files_under, read_tree, rustfmt, write_tree};
use crate::roots::{NotAStructure, V2RootSet};
use crate::v2::corpus::{Corpus, LoadError};
use crate::v2::lower::{LowerError, Model};
use crate::v2::render::{RenderError, render};

/// What to emit and where.
#[derive(Debug, Clone)]
pub struct EmitOptions {
    /// The fetched v2 definitions (the directory holding `PROVENANCE.md` and `input/`).
    pub definitions: PathBuf,
    /// The generated crate directory (the one holding `Cargo.toml` and `src/`).
    pub crate_dir: PathBuf,
    /// Compare instead of writing.
    pub check: bool,
}

/// What an emit run produced.
#[derive(Debug)]
pub struct EmitReport {
    /// The number of message structures.
    pub structures: usize,
    /// The number of segments.
    pub segments: usize,
    /// The number of segment fields.
    pub fields: usize,
    /// The files written or checked, relative to `src/`.
    pub files: Vec<String>,
}

/// A failure while emitting `hl7v2-types`.
#[derive(Debug, thiserror::Error)]
pub enum EmitError {
    /// The definitions did not load.
    #[error(transparent)]
    Load(#[from] LoadError),
    /// The structure directory holds something other than a message structure.
    #[error(transparent)]
    Root(#[from] NotAStructure),
    /// Lowering failed.
    #[error(transparent)]
    Lower(#[from] LowerError),
    /// Rendering failed.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// Writing, formatting, or checking the tree failed.
    #[error(transparent)]
    Tree(#[from] crate::emit::EmitError),
}

/// Runs the v2 pipeline per `options`.
///
/// # Errors
///
/// Returns [`EmitError`] for a load, lowering, render, I/O, or `rustfmt`
/// failure, and a [`crate::emit::EmitError::Drift`] in check mode when the tree differs.
pub fn emit(options: &EmitOptions) -> Result<EmitReport, EmitError> {
    let corpus = Corpus::open(&options.definitions)?;
    let roots = V2RootSet::select(&corpus)?;
    let model = Model::lower(&corpus, &roots)?;
    let files = render(&model)?;

    let scratch = tempfile::tempdir().map_err(|source| crate::emit::EmitError::Io {
        path: PathBuf::from("(temporary directory)"),
        source,
    })?;
    let scratch_src = scratch.path().join("src");
    write_tree(&scratch_src, &files)?;
    rustfmt(&scratch_src, &files, &options.crate_dir)?;
    let formatted = read_tree(&scratch_src, &files)?;

    let target_src = options.crate_dir.join("src");
    if options.check {
        let mut drift = Vec::new();
        for (relative, content) in &formatted {
            match fs::read_to_string(target_src.join(relative)) {
                Ok(existing) if existing == *content => {}
                Ok(_) => drift.push(relative.clone()),
                Err(_) => drift.push(format!("{relative} (missing)")),
            }
        }
        if target_src.is_dir() {
            for relative in files_under(&target_src, "")? {
                let relative = relative.trim_start_matches('/').to_owned();
                if !formatted.contains_key(&relative) {
                    drift.push(format!("{relative} (stale)"));
                }
            }
        }
        drift.sort();
        if !drift.is_empty() {
            return Err(crate::emit::EmitError::Drift { paths: drift }.into());
        }
    } else {
        if target_src.exists() {
            fs::remove_dir_all(&target_src).map_err(|source| crate::emit::EmitError::Io {
                path: target_src.clone(),
                source,
            })?;
        }
        write_tree(&target_src, &formatted)?;
    }
    Ok(EmitReport {
        structures: model.structures.len(),
        segments: model.segments.len(),
        fields: model.field_count(),
        files: formatted.keys().cloned().collect(),
    })
}
