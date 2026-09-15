// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Loading the mapping set and the templates it compiles against.
//!
//! A FHIRconnect context is compiled once, at boot, against the operational
//! template it names, so a mapping that does not resolve refuses the start
//! rather than the request that first touches it. No specification governs
//! where the files live: our own design.

use std::path::Path;
use std::path::PathBuf;

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::model::load::load_set;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::operations::programs::ProgramSet;
use fhirconnect::resolve::compile::compile;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::TemplateSource;

use crate::config::MappingSettings;

/// The extension an operational template file carries.
const TEMPLATE_EXTENSION: &str = "opt";

/// The extensions a mapping file carries.
const MAPPING_EXTENSIONS: [&str; 2] = ["yml", "yaml"];

/// Why a mapping set did not load.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A configured directory could not be read.
    #[error("cannot read {}", path.display())]
    Read {
        /// The directory that could not be read.
        path: PathBuf,
        /// What the file system reported.
        #[source]
        source: std::io::Error,
    },
    /// A directory holds none of the files the lane needs.
    #[error("{} holds no {what}", path.display())]
    Empty {
        /// The directory that was searched.
        path: PathBuf,
        /// What was looked for.
        what: &'static str,
    },
    /// An operational template did not build a Web Template.
    #[error("the operational template {} did not load", path.display())]
    Template {
        /// The file that was refused.
        path: PathBuf,
        /// Why it was refused.
        #[source]
        source: Box<openehr_mapping_core::template::PathError>,
    },
    /// The mapping files did not pass the validation layers.
    #[error("the mapping set under {} is not valid: {report}", path.display())]
    Mappings {
        /// The directory the files came from.
        path: PathBuf,
        /// One line per refusal.
        report: String,
    },
    /// A context mapping did not compile into a program.
    #[error("the context `{context}` did not compile: {report}")]
    Compile {
        /// The context mapping that was refused.
        context: String,
        /// One line per refusal.
        report: String,
    },
    /// A context mapping names a template the configured directory does not
    /// hold.
    #[error("the context `{context}` maps `{template}`, which no loaded template provides")]
    UnknownTemplate {
        /// The context mapping that named it.
        context: String,
        /// The template it named.
        template: String,
    },
    /// A compiled program did not join the set.
    #[error("the context `{context}` did not join the compiled set")]
    Insert {
        /// The context mapping that was refused.
        context: String,
        /// Why it was refused.
        #[source]
        source: fhirconnect::operations::programs::ProgramSetError,
    },
}

/// Loads the mapping set `settings` names, compiled and ready to run.
///
/// # Errors
///
/// Returns [`Error`] when a directory cannot be read or holds nothing, when a
/// template does not build, when the mapping files do not validate, and when a
/// context does not compile against its template.
pub fn load(settings: &MappingSettings) -> Result<ProgramSet, Error> {
    // TODO(#195): take the operational templates from the CDR when `[cdr]` is
    // configured, instead of the directory alone.
    let mut set = ProgramSet::new();
    let mut indices: Vec<WebTemplateIndex> = Vec::new();
    for path in files(&settings.templates, &[TEMPLATE_EXTENSION])? {
        let xml = std::fs::read_to_string(&path).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?;
        let source = TemplateSource::opt14(&xml).map_err(|error| Error::Template {
            path: path.clone(),
            source: Box::new(error),
        })?;
        let index = WebTemplateIndex::build(&source).map_err(|error| Error::Template {
            path: path.clone(),
            source: Box::new(error),
        })?;
        indices.push(index);
    }
    if indices.is_empty() {
        return Err(Error::Empty {
            path: settings.templates.clone(),
            what: "operational template",
        });
    }
    for index in indices {
        set.insert_template(index);
    }

    let mapping_files = files(&settings.directory, &MAPPING_EXTENSIONS)?;
    if mapping_files.is_empty() {
        return Err(Error::Empty {
            path: settings.directory.clone(),
            what: "mapping file",
        });
    }
    let loaded =
        load_set(&mapping_files, &StaticMappingCodes::default()).map_err(|diagnostics| {
            Error::Mappings {
                path: settings.directory.clone(),
                report: report(&diagnostics),
            }
        })?;
    for context in loaded.contexts() {
        let name = context.header().name().value().clone();
        let template = context
            .context()
            .template
            .id
            .as_ref()
            .map(|located| located.value().clone())
            .unwrap_or_default();
        let index = set
            .template(&fhirconnect::resolve::program::TemplateId::new(
                template.clone(),
            ))
            .ok_or_else(|| Error::UnknownTemplate {
                context: name.as_str().to_owned(),
                template: template.clone(),
            })?;
        let program = compile(
            &loaded,
            &name,
            index,
            &SCHEMAS,
            &StaticMappingCodes::default(),
        )
        .map_err(|diagnostics| Error::Compile {
            context: name.as_str().to_owned(),
            report: report(&diagnostics),
        })?;
        set.insert_program(program)
            .map_err(|source| Error::Insert {
                context: name.as_str().to_owned(),
                source,
            })?;
    }
    Ok(set)
}

/// Returns every file under `directory` whose extension is one of `wanted`.
///
/// The walk is one level deep and the result is sorted, so two deployments
/// with the same tree load the same set in the same order.
fn files(directory: &Path, wanted: &[&str]) -> Result<Vec<PathBuf>, Error> {
    let mut found = Vec::new();
    let mut stack = vec![directory.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|source| Error::Read {
            path: current.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| Error::Read {
                path: current.clone(),
                source,
            })?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let matches = path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| wanted.contains(&extension));
            if matches {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Renders a diagnostic list as one line.
fn report(diagnostics: &[openehr_mapping_core::diagnostic::Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{} {}: {}",
                diagnostic.file().display(),
                diagnostic.code(),
                diagnostic.message()
            )
        })
        .collect::<Vec<String>>()
        .join("; ")
}
