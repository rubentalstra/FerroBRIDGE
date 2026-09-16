// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `omop-cdm-codegen` binary: parse the command line, run the generator.
#![expect(clippy::print_stdout, reason = "a command-line tool reports to stdout")]

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = omop_cdm_codegen::Cli::parse();
    match omop_cdm_codegen::run(&cli)? {
        omop_cdm_codegen::Report::Emit(report) => {
            let columns: usize = report.columns.values().sum();
            println!(
                "omop-cdm-codegen: {} tables, {columns} columns, {} files",
                report.columns.len(),
                report.files.len()
            );
        }
    }
    Ok(())
}
