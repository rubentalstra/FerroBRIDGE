// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FerroBRIDGE server library: the run path the `ferrobridge` binary and
//! its integration tests share.
//!
//! The binary is one, with subcommands; this release carries the workspace
//! skeleton only.
#![doc(test(attr(deny(warnings))))]

use std::process::ExitCode;

/// Runs the server with the given arguments and returns the process exit code.
///
/// The subcommands (`serve`, `etl`, `cdm`, `vocab`, `mapping`) land with the
/// server shape; until then every invocation exits successfully having done
/// nothing.
#[must_use]
pub fn run<I>(_args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    // TODO(#21): the subcommands and the run path.
    ExitCode::SUCCESS
}
