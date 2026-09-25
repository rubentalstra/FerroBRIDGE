// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `fhir-codegen` binary: parse the command line, run the generator.
#![expect(clippy::print_stdout, reason = "a command-line tool reports to stdout")]

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = fhir_codegen::Cli::parse();
    match fhir_codegen::run(&cli)? {
        fhir_codegen::Report::Emit(report, hl7v2) => {
            for (module, types) in &report.types {
                println!("fhir-codegen: {module}: {types} types");
            }
            println!("fhir-codegen: {} files", report.files.len());
            println!(
                "fhir-codegen: hl7v2-types: {} message structures, {} segments, {} fields, {} data types, {} components, {} messages, {} legacy structures of {} codes, {} legacy segments, {} shared segment references, {} files",
                hl7v2.structures,
                hl7v2.segments,
                hl7v2.fields,
                hl7v2.data_types,
                hl7v2.components,
                hl7v2.messages,
                hl7v2.legacy_structures,
                hl7v2.legacy_codes,
                hl7v2.legacy_segments,
                hl7v2.legacy_shared,
                hl7v2.files.len()
            );
        }
        fhir_codegen::Report::Terminology(report) => {
            for (module, counts) in &report.counts {
                println!(
                    "fhir-codegen: {module}: {} code systems, {} value sets, {} without a status, {} with a duplicate code",
                    counts.code_systems,
                    counts.value_sets,
                    counts.without_status,
                    counts.with_duplicate_code
                );
            }
        }
    }
    Ok(())
}
