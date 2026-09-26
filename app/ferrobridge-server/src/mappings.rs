// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Loading the mapping set and the templates it compiles against.
//!
//! A FHIRconnect context is compiled once, at boot, against the operational
//! template it names, so a mapping that does not resolve refuses the start
//! rather than the request that first touches it. The templates come from
//! `[mappings] templates` when it names a directory, and otherwise from the
//! CDR, which serves each one at `GET /definition/template/adl1.4/{id}`
//! (openEHR ITS-REST 1.1.0 §Definition). No specification governs where the
//! files live: our own design.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::model::load::MappingSet;
use fhirconnect::model::load::load_set;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::operations::programs::ProgramSet;
use fhirconnect::resolve::compile::compile;
use http::StatusCode;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::TemplateSource;

use crate::config::MappingSettings;
use crate::config::Settings;

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
    /// The operations lane has a mapping directory and nowhere to take its
    /// templates from.
    #[error(
        "[mappings] directory {} is set with no [mappings] templates and no [cdr]: the FHIRconnect operations have no template to compile against",
        directory.display()
    )]
    NoTemplateSource {
        /// The mapping directory that was configured.
        directory: PathBuf,
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
    /// A template the CDR served did not build a Web Template.
    #[error("the template `{template}` the CDR served did not load")]
    CdrTemplate {
        /// The template identifier that was fetched.
        template: String,
        /// Why it was refused.
        #[source]
        source: Box<openehr_mapping_core::template::PathError>,
    },
    /// A context names a template identifier the CDR route cannot carry.
    #[error("`{template}` is no template identifier the CDR route can carry")]
    TemplateName {
        /// The identifier a context wrote.
        template: String,
        /// What the identifier's constructor reported.
        #[source]
        source: Box<crate::cdr::ids::IdError>,
    },
    /// The template fetch did not reach a documented answer.
    #[error("the CDR did not serve the template `{template}`")]
    TemplateFetch {
        /// The template identifier that was asked for.
        template: String,
        /// What the client reported, the upstream status included.
        #[source]
        source: Box<crate::cdr::error::CdrError>,
    },
    /// The CDR holds no template with the identifier a context names.
    #[error("the CDR holds no template `{template}`: it answered {status}")]
    TemplateNotHeld {
        /// The template identifier that was asked for.
        template: String,
        /// The status the CDR answered.
        status: StatusCode,
    },
    /// The CDR refused the template fetch.
    #[error("the CDR refused the template `{template}`: it answered {upstream}")]
    TemplateRefused {
        /// The template identifier that was asked for.
        template: String,
        /// The upstream status and body.
        upstream: Box<crate::cdr::error::Upstream>,
    },
    /// The mapping files did not pass the validation layers.
    #[error("the mapping set under {} is not valid: {report}", path.display())]
    Mappings {
        /// The directory the files came from.
        path: PathBuf,
        /// One line per refusal.
        report: String,
    },
    /// A context mapping names no template identifier.
    #[error("the context `{context}` names no template id")]
    NoTemplateId {
        /// The context mapping that names none.
        context: String,
    },
    /// A context mapping did not compile into a program.
    #[error("the context `{context}` did not compile: {report}")]
    Compile {
        /// The context mapping that was refused.
        context: String,
        /// One line per refusal.
        report: String,
    },
    /// A context mapping names a template no loaded template provides.
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

/// Loads the mapping set the operations lane of `settings` runs, if any.
///
/// The lane is off when `[operations] enabled` is false or no
/// `[mappings] directory` is set. Otherwise the templates come from
/// `[mappings] templates` when it is set, and from the CDR `cdr` reaches when
/// it is not.
///
/// # Errors
///
/// Returns [`Error::NoTemplateSource`] when a mapping directory is set with
/// neither a template directory nor a CDR, and the errors of [`load`] and
/// [`load_from_cdr`] otherwise.
pub async fn load_configured(
    settings: &Settings,
    cdr: Option<&crate::cdr::CdrClient>,
) -> Result<Option<ProgramSet>, Error> {
    if !settings.operations.enabled {
        return Ok(None);
    }
    // NOTE: no specification governs this: our own design; a template
    // directory is the operator's explicit choice, so it wins over the CDR.
    if let Some(ref mappings) = settings.mappings {
        return load(mappings).map(Some);
    }
    match (settings.mapping_directory.as_deref(), cdr) {
        (Some(directory), Some(client)) => load_from_cdr(directory, client).await.map(Some),
        (Some(directory), None) => Err(Error::NoTemplateSource {
            directory: directory.to_path_buf(),
        }),
        (None, _) => Ok(None),
    }
}

/// Loads the mapping set `settings` names, compiled and ready to run.
///
/// Every `.opt` file under the template directory is read, and each context
/// compiles against the one it names.
///
/// # Errors
///
/// Returns [`Error`] when a directory cannot be read or holds nothing, when a
/// template does not build, when the mapping files do not validate, and when a
/// context does not compile against its template.
pub fn load(settings: &MappingSettings) -> Result<ProgramSet, Error> {
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
    let loaded = read_mappings(&settings.directory)?;
    compile_all(&mut set, &loaded)?;
    Ok(set)
}

/// Loads the mapping set under `directory`, compiled against the templates
/// the CDR `client` reaches serves.
///
/// The context files are read first, and every template identifier they name
/// is fetched once, in order, before any context compiles. A template the CDR
/// does not serve refuses the whole set, so the lane never starts half
/// loaded.
///
/// # Errors
///
/// Returns [`Error::Read`], [`Error::Empty`] and [`Error::Mappings`] for the
/// mapping files, [`Error::NoTemplateId`] for a context naming no template,
/// [`Error::TemplateName`], [`Error::TemplateFetch`],
/// [`Error::TemplateNotHeld`], [`Error::TemplateRefused`] and
/// [`Error::CdrTemplate`] for a template the CDR
/// does not serve as a Web Template, and the compile errors of [`load`].
pub async fn load_from_cdr(
    directory: &Path,
    client: &crate::cdr::CdrClient,
) -> Result<ProgramSet, Error> {
    let loaded = read_mappings(directory)?;
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for context in loaded.contexts() {
        wanted.insert(template_of(context)?);
    }
    let mut set = ProgramSet::new();
    for template in wanted {
        set.insert_template(template_index(client, &template).await?);
    }
    compile_all(&mut set, &loaded)?;
    Ok(set)
}

/// Fetches the template `template` names from the CDR and builds its Web
/// Template index.
///
/// This is the one CDR template fetch at boot, which the operations lane and
/// the facade both call.
///
/// # Errors
///
/// Returns [`Error::TemplateName`] for an identifier the route cannot carry,
/// [`Error::TemplateNotHeld`] with the status when the CDR holds no such
/// template, [`Error::TemplateRefused`] with the upstream answer for a `400`,
/// [`Error::TemplateFetch`] when the call reaches no documented answer, and
/// [`Error::CdrTemplate`] when the template does not build a Web Template.
pub async fn template_index(
    client: &crate::cdr::CdrClient,
    template: &str,
) -> Result<WebTemplateIndex, Error> {
    let source = fetch(client, template).await?;
    WebTemplateIndex::build(&source).map_err(|error| Error::CdrTemplate {
        template: template.to_owned(),
        source: Box::new(error),
    })
}

/// Fetches the template `template` names from the CDR.
async fn fetch(client: &crate::cdr::CdrClient, template: &str) -> Result<TemplateSource, Error> {
    let id = crate::cdr::ids::template_id(template).map_err(|source| Error::TemplateName {
        template: template.to_owned(),
        source: Box::new(source),
    })?;
    let outcome = client
        .template(&id)
        .await
        .map_err(|source| Error::TemplateFetch {
            template: template.to_owned(),
            source: Box::new(source),
        })?;
    match outcome {
        crate::cdr::template::TemplateOutcome::Found(
            crate::cdr::template::TemplateSource::Opt14(opt),
        ) => Ok(TemplateSource::Opt14(opt)),
        crate::cdr::template::TemplateOutcome::Found(
            crate::cdr::template::TemplateSource::Opt2 {
                template: opt,
                resolved_id,
            },
        ) => Ok(TemplateSource::Opt2 {
            template: opt,
            resolved_id: resolved_id.physical_id(),
        }),
        // NOTE: the client reports `UnknownTemplate` only for a `404` from both
        // definition routes (`definition-codegen.openapi.yaml`, ITS-REST 1.1.0).
        crate::cdr::template::TemplateOutcome::UnknownTemplate => Err(Error::TemplateNotHeld {
            template: template.to_owned(),
            status: StatusCode::NOT_FOUND,
        }),
        crate::cdr::template::TemplateOutcome::BadRequest(upstream) => {
            Err(Error::TemplateRefused {
                template: template.to_owned(),
                upstream: Box::new(upstream),
            })
        }
    }
}

/// Reads and validates the mapping files under `directory`.
fn read_mappings(directory: &Path) -> Result<MappingSet, Error> {
    let mapping_files = files(directory, &MAPPING_EXTENSIONS)?;
    if mapping_files.is_empty() {
        return Err(Error::Empty {
            path: directory.to_path_buf(),
            what: "mapping file",
        });
    }
    load_set(&mapping_files, &StaticMappingCodes::default()).map_err(|diagnostics| {
        Error::Mappings {
            path: directory.to_path_buf(),
            report: report(&diagnostics),
        }
    })
}

/// Logs every warning a compiled program carries, once, as it loads.
///
/// A warning is a disagreement the compiler accepted, so the program runs and
/// the operator learns of it here. The record names the context, the file,
/// the line and the diagnostic code, and never the content of a mapping.
pub fn log_warnings(program: &fhirconnect::resolve::program::Program) {
    for warning in program.warnings() {
        tracing::warn!(
            context = %program.context(),
            file = %warning.file().display(),
            line = warning
                .position()
                .map_or(0, openehr_mapping_core::position::Position::line),
            code = %warning.code(),
            "a mapping context compiled with a warning"
        );
    }
}

/// Compiles every context of `loaded` against the templates `set` holds.
fn compile_all(set: &mut ProgramSet, loaded: &MappingSet) -> Result<(), Error> {
    for context in loaded.contexts() {
        let name = context.header().name().value().clone();
        let template = template_of(context)?;
        let index = set
            .template(&fhirconnect::resolve::program::binding::TemplateId::new(
                template.clone(),
            ))
            .ok_or_else(|| Error::UnknownTemplate {
                context: name.as_str().to_owned(),
                template: template.clone(),
            })?;
        let program = compile(
            loaded,
            &name,
            index,
            &SCHEMAS,
            &StaticMappingCodes::default(),
        )
        .map_err(|diagnostics| Error::Compile {
            context: name.as_str().to_owned(),
            report: report(&diagnostics),
        })?;
        log_warnings(&program);
        set.insert_program(program)
            .map_err(|source| Error::Insert {
                context: name.as_str().to_owned(),
                source,
            })?;
    }
    Ok(())
}

/// Returns the template identifier one context names.
fn template_of(context: &fhirconnect::model::ast::ContextMappingFile) -> Result<String, Error> {
    context
        .context()
        .template
        .id
        .as_ref()
        .map(|located| located.value().clone())
        .ok_or_else(|| Error::NoTemplateId {
            context: context.header().name().value().as_str().to_owned(),
        })
}

/// Returns every file under `directory` whose extension is one of `wanted`.
///
/// The walk descends into every subdirectory and the result is sorted, so two
/// deployments with the same tree load the same set in the same order.
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
