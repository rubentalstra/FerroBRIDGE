// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The command line: one binary, one subcommand per job.
//!
//! `serve` runs the server; the batch jobs share the same configuration and
//! the same run path. No specification governs the command line: our own
//! design.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// The `ferrobridge` command line.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(
    name = "ferrobridge",
    version,
    about = "The FerroBRIDGE bridge between openEHR, HL7 FHIR and the OMOP CDM"
)]
pub struct Cli {
    /// The configuration file to read, overriding `FERROBRIDGE_CONFIG`.
    #[arg(long, value_name = "PATH", global = true)]
    pub config: Option<PathBuf>,
    /// The job to run.
    #[command(subcommand)]
    pub command: Command,
}

/// What the binary does.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Serves the HTTP surface until the process is asked to stop.
    Serve,
    /// Runs an OMOP extract, transform and load job.
    Etl {
        /// The job to run.
        #[command(subcommand)]
        command: Etl,
    },
    /// Works on the OMOP CDM database.
    Cdm {
        /// The job to run.
        #[command(subcommand)]
        command: Cdm,
    },
    /// Works on the OHDSI vocabulary.
    Vocab {
        /// The job to run.
        #[command(subcommand)]
        command: Vocab,
    },
    /// Works on the mapping files.
    Mapping {
        /// The job to run.
        #[command(subcommand)]
        command: Mapping,
    },
}

/// The `etl` jobs.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Etl {
    /// Loads compositions from the CDR into the CDM database.
    Run {
        /// Skips every composition whose watermark names the version the
        /// query answers.
        #[arg(long)]
        resume: bool,
        /// Binds the value to the composition query's `$since` parameter.
        #[arg(long, value_name = "TIME")]
        since: Option<String>,
    },
}

/// The `cdm` jobs.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Cdm {
    /// Applies the OMOP CDM v5.4 DDL and the bridge schema to the configured
    /// database.
    Init,
}

/// The `vocab` jobs.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Vocab {
    /// Loads an OHDSI vocabulary export into the configured database.
    Load {
        /// The directory holding the export.
        #[arg(value_name = "DIR")]
        directory: PathBuf,
        /// The schema the vocabulary tables live in.
        #[arg(long, value_name = "NAME")]
        schema: String,
    },
}

/// The `mapping` jobs.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Mapping {
    /// Validates the configured mapping files and reports every diagnostic.
    Check,
}

impl Command {
    /// Returns the tracker issue that lands this job, or `None` when it runs.
    ///
    /// A job that parses and does not run yet names the issue that carries
    /// it, so an operator reads one line instead of a silent success.
    #[must_use]
    pub const fn pending_issue(&self) -> Option<u32> {
        match self {
            Self::Serve | Self::Cdm { .. } => None,
            Self::Etl { .. } => Some(90),
            Self::Vocab { .. } => Some(233),
            Self::Mapping { .. } => Some(82),
        }
    }

    /// Returns the job as it is spelled on the command line.
    #[must_use]
    pub const fn spelling(&self) -> &'static str {
        match self {
            Self::Serve => "serve",
            Self::Etl {
                command: Etl::Run { .. },
            } => "etl run",
            Self::Cdm { command: Cdm::Init } => "cdm init",
            Self::Vocab {
                command: Vocab::Load { .. },
            } => "vocab load",
            Self::Mapping {
                command: Mapping::Check,
            } => "mapping check",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cdm, Cli, Command, Etl, Mapping, Vocab};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn every_documented_subcommand_parses() {
        let cases: [(&[&str], Command); 7] = [
            (&["ferrobridge", "serve"], Command::Serve),
            (
                &["ferrobridge", "etl", "run"],
                Command::Etl {
                    command: Etl::Run {
                        resume: false,
                        since: None,
                    },
                },
            ),
            (
                &["ferrobridge", "etl", "run", "--resume"],
                Command::Etl {
                    command: Etl::Run {
                        resume: true,
                        since: None,
                    },
                },
            ),
            (
                &[
                    "ferrobridge",
                    "etl",
                    "run",
                    "--since",
                    "2026-09-01T00:00:00Z",
                ],
                Command::Etl {
                    command: Etl::Run {
                        resume: false,
                        since: Some(String::from("2026-09-01T00:00:00Z")),
                    },
                },
            ),
            (
                &["ferrobridge", "cdm", "init"],
                Command::Cdm { command: Cdm::Init },
            ),
            (
                &[
                    "ferrobridge",
                    "vocab",
                    "load",
                    "/srv/athena",
                    "--schema",
                    "cdm",
                ],
                Command::Vocab {
                    command: Vocab::Load {
                        directory: PathBuf::from("/srv/athena"),
                        schema: String::from("cdm"),
                    },
                },
            ),
            (
                &["ferrobridge", "mapping", "check"],
                Command::Mapping {
                    command: Mapping::Check,
                },
            ),
        ];
        for (argv, expected) in cases {
            let cli = Cli::try_parse_from(argv).expect("the subcommand parses");
            assert_eq!(expected, cli.command, "{argv:?}");
            assert_eq!(None, cli.config, "no --config was given");
        }
    }

    #[test]
    fn the_config_flag_is_global_and_reads_a_path() {
        let cli = Cli::try_parse_from(["ferrobridge", "serve", "--config", "/etc/fb.toml"])
            .expect("--config parses after the subcommand");
        assert_eq!(Some(PathBuf::from("/etc/fb.toml")), cli.config);
    }

    #[test]
    fn a_subcommand_is_required_and_an_unknown_one_is_refused() {
        assert!(
            Cli::try_parse_from(["ferrobridge"]).is_err(),
            "the binary does nothing without a job"
        );
        assert!(
            Cli::try_parse_from(["ferrobridge", "run"]).is_err(),
            "an unknown word must not fall through to serving"
        );
        assert!(
            Cli::try_parse_from(["ferrobridge", "etl"]).is_err(),
            "a job group needs its job"
        );
        assert!(
            Cli::try_parse_from(["ferrobridge", "vocab", "load"]).is_err(),
            "the loader needs a directory and a schema"
        );
    }

    #[test]
    fn a_pending_job_names_the_issue_that_lands_it() {
        assert_eq!(None, Command::Serve.pending_issue());
        assert_eq!(None, Command::Cdm { command: Cdm::Init }.pending_issue());
        let etl = Command::Etl {
            command: Etl::Run {
                resume: false,
                since: None,
            },
        };
        assert_eq!(
            Some(90),
            etl.pending_issue(),
            "the runner waits on the OMOCL engine"
        );
        assert_eq!("etl run", etl.spelling());
        assert_eq!(
            Some(233),
            Command::Vocab {
                command: Vocab::Load {
                    directory: PathBuf::from("/srv/athena"),
                    schema: String::from("cdm"),
                },
            }
            .pending_issue(),
            "the loader waits on the observed export format"
        );
        assert_eq!(
            Some(82),
            Command::Mapping {
                command: Mapping::Check
            }
            .pending_issue()
        );
    }
}
