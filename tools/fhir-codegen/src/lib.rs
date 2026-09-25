// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `fhir-types` and `hl7v2-types` generator.
//!
//! Reads the vendored, pinned HL7 FHIR packages under `vendor/` and emits the
//! per-version Rust modules of `crates/fhir-types`: the terminology root-set
//! types and the terminology operation contracts. Reads the HL7 v2 definitions
//! fetched into `vendor/hl7-v2ig/` and emits `crates/hl7v2-types` ([`v2`]).
//! The output is byte-deterministic so the CI drift check can regenerate and
//! compare.
//!
//! The pipeline is [`package::Package`] (read a package), then
//! [`snapshot::ResolvedStructure`] (resolve each structure's snapshot),
//! [`roots::RootSet`] (select what to emit), [`closure::TypeClosure`] (the
//! root-set closure), [`lower::VersionModule`] (the generated module), [`render`]
//! (source text), and [`emit`] (write or check).
#![doc(test(attr(deny(warnings))))]

pub mod closure;
pub mod ecosystem;
pub mod emit;
pub mod fhir;
pub mod lower;
pub mod naming;
pub mod operations;
pub mod package;
pub mod render;
pub mod render_codec;
pub mod render_schema;
pub mod roots;
pub mod snapshot;
pub mod terminology;
pub mod v2;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

/// The FHIR versions the generator emits: module name and vendored package.
///
/// Every entry is emitted on each run so the generated crate always carries
/// the whole set (`codegen.md`).
pub const VERSIONS: [(&str, &str); 4] = [
    ("r4", "hl7.fhir.r4.core"),
    ("r4b", "hl7.fhir.r4b.core"),
    ("r5", "hl7.fhir.r5.core"),
    ("r6", "hl7.fhir.r6.core"),
];

/// The command line of `fhir-codegen`.
#[derive(Debug, Parser)]
#[command(name = "fhir-codegen", version, about)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The generator's subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Regenerates `crates/fhir-types` from the vendored packages and
    /// `crates/hl7v2-types` from the fetched HL7 v2 definitions.
    Emit {
        /// Compare the generated crates with what the emitter produces and fail on any difference.
        #[arg(long)]
        check: bool,
        /// The directory holding the vendored packages and, under `hl7-v2ig/`, the v2 definitions.
        #[arg(long, value_name = "DIR", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor"))]
        vendor: PathBuf,
        /// The generated FHIR crate directory.
        #[arg(long, value_name = "DIR", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/fhir-types"))]
        out: PathBuf,
        /// The generated HL7 v2 crate directory.
        #[arg(long, value_name = "DIR", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/hl7v2-types"))]
        hl7v2_out: PathBuf,
    },
    /// Regenerates the FHIR core terminology bundles a terminology server embeds.
    Terminology {
        /// Compare the bundles on disk with what the emitter produces and fail on any difference.
        #[arg(long)]
        check: bool,
        /// The directory holding the vendored packages.
        #[arg(long, value_name = "DIR", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor"))]
        vendor: PathBuf,
        // NOTE: no specification governs this: our own design, and the
        // consumer of these bundles is the sibling terminology server, so the
        // caller names the directory.
        /// The directory holding one bundle directory per version module.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
}

/// A generator failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The emit pipeline failed.
    #[error(transparent)]
    Emit(#[from] emit::EmitError),
    /// The v2 emit pipeline failed.
    #[error(transparent)]
    V2(#[from] v2::emit::EmitError),
    /// The terminology bundles failed.
    #[error(transparent)]
    Terminology(#[from] terminology::BundleError),
}

/// What one run of the generator produced.
#[derive(Debug)]
pub enum Report {
    /// The two generated crates, `fhir-types` first.
    Emit(emit::EmitReport, v2::emit::EmitReport),
    /// The FHIR core terminology bundles.
    Terminology(terminology::BundleReport),
}

/// The emit inputs for every entry of [`VERSIONS`] under `vendor`.
#[must_use]
pub fn version_inputs(vendor: &Path) -> Vec<emit::VersionInput> {
    VERSIONS
        .iter()
        .map(|(module, package)| emit::VersionInput {
            module: (*module).to_owned(),
            package_dir: vendor.join(package),
        })
        .collect()
}

/// Runs the command the CLI selected.
///
/// # Errors
///
/// Returns [`Error::Emit`], [`Error::V2`] or [`Error::Terminology`] when the
/// pipeline fails or, in check mode, when what is on disk differs from what
/// the emitter produces.
pub fn run(cli: &Cli) -> Result<Report, Error> {
    match &cli.command {
        Command::Emit {
            check,
            vendor,
            out,
            hl7v2_out,
        } => {
            let fhir = emit::emit(&emit::EmitOptions {
                versions: version_inputs(vendor),
                crate_dir: out.clone(),
                check: *check,
            })?;
            let hl7v2 = v2::emit::emit(&v2::emit::EmitOptions {
                definitions: vendor.join("hl7-v2ig"),
                crate_dir: hl7v2_out.clone(),
                check: *check,
            })?;
            Ok(Report::Emit(fhir, hl7v2))
        }
        Command::Terminology { check, vendor, out } => Ok(Report::Terminology(
            terminology::bundle(&terminology::BundleOptions {
                versions: version_inputs(vendor),
                data_dir: out.clone(),
                check: *check,
            })?,
        )),
    }
}
