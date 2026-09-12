// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The `omop-cdm` generator.
//!
//! Reads the vendored OMOP CDM v5.4 definitions under
//! `docs/specs/omop-cdm/inst/csv/` and emits `crates/omop-cdm/src/generated/`:
//! one module per table with its row type and its column metadata, plus the
//! four rendered PostgreSQL DDL files copied in for `include_str!`. The output
//! is byte-deterministic so the CI drift check can regenerate and compare.
//!
//! The pipeline is [`definitions::Definitions`] (read the two CSV files),
//! [`lower::Model`] (the Rust model), [`render`] (source text) and [`emit`]
//! (write or check).
#![doc(test(attr(deny(warnings))))]

pub mod definitions;
pub mod emit;
pub mod lower;
pub mod render;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// The command line of `omop-cdm-codegen`.
#[derive(Debug, Parser)]
#[command(name = "omop-cdm-codegen", version, about)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The generator's subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Regenerates `crates/omop-cdm` from the vendored CDM definitions.
    Emit {
        /// Compare the generated tree with what the emitter produces and fail on any difference.
        #[arg(long)]
        check: bool,
        /// The directory holding the two vendored definition CSV files.
        #[arg(long, value_name = "DIR", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/specs/omop-cdm/inst/csv"))]
        definitions: PathBuf,
        /// The directory holding the four vendored PostgreSQL DDL files.
        #[arg(long, value_name = "DIR", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/specs/omop-cdm/inst/ddl/5.4/postgresql"))]
        ddl: PathBuf,
        /// The generated crate directory.
        #[arg(long, value_name = "DIR", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/omop-cdm"))]
        out: PathBuf,
        /// The `rustfmt.toml` the generated source is formatted with.
        #[arg(long, value_name = "FILE", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rustfmt.toml"))]
        rustfmt_config: PathBuf,
    },
}

/// A generator failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The emit pipeline failed.
    #[error(transparent)]
    Emit(#[from] emit::EmitError),
}

/// What one run of the generator produced.
#[derive(Debug)]
pub enum Report {
    /// The generated tree.
    Emit(emit::EmitReport),
}

/// Runs the command the CLI selected.
///
/// # Errors
///
/// Returns [`Error::Emit`] when the pipeline fails or, in check mode, when
/// what is on disk differs from what the emitter produces.
pub fn run(cli: &Cli) -> Result<Report, Error> {
    match &cli.command {
        Command::Emit {
            check,
            definitions,
            ddl,
            out,
            rustfmt_config,
        } => Ok(Report::Emit(emit::emit(&emit::EmitOptions {
            definitions_dir: definitions.clone(),
            ddl_dir: ddl.clone(),
            crate_dir: out.clone(),
            rustfmt_config: rustfmt_config.clone(),
            check: *check,
        })?)),
    }
}
