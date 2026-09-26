// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL mapper `etl run` maps every composition with.
//!
//! The OMOCL files are read and validated once, at boot, from `[mappings]
//! omocl`, so a file that does not load refuses the start. OMOCL maps by
//! archetype and names no template, so a program is compiled the first time a
//! composition of a template arrives, against the template the runner read
//! from the CDR, and kept for the rest of the run; a set that does not compile
//! against a template refuses each composition of it. The vocabulary is the
//! concept resolver of `omop-cdm` over the CDM database. No specification
//! governs this wiring: our own design.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use omocl::engine::Seams;
use omocl::engine::concept::ConceptSource;
use omocl::engine::concept::LookupError;
use omocl::engine::concept::Operator;
use omocl::engine::concept::VocabularyAliases;
use omocl::model::ast::ConceptId;
use omocl::model::load::MappingSet;
use omocl::model::semantic::FirstPartyConverters;
use omocl::resolve::program::Program;
use omop_cdm::graph::RecordGraph;
use omop_cdm::graph::report::Refusal;
use omop_cdm::value::CdmDate;
use omop_cdm::vocabulary::ConceptResolver;
use omop_cdm::vocabulary::Resolution;
use omop_cdm::vocabulary::ResolveError;
use omop_cdm::vocabulary::SourceKey;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::index::WebTemplateIndex;

use crate::etl::Mapper;
use crate::etl::SourceComposition;

/// Why the OMOCL mapping set could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MapperError {
    /// The directory or a file in it could not be read.
    #[error("reading the OMOCL directory {}", path.display())]
    Read {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The directory holds no OMOCL file.
    #[error("the OMOCL directory {} holds no .yml or .yaml file", path.display())]
    Empty {
        /// The directory.
        path: PathBuf,
    },
    /// A file did not load or validate.
    #[error("the OMOCL mapping set does not load: {}", render(diagnostics))]
    Mappings {
        /// Every diagnostic the loader raised.
        diagnostics: Vec<Diagnostic>,
    },
}

/// Renders a diagnostic list on one line.
fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join("; ")
}

/// Collects every `.yml` and `.yaml` file below `directory`.
fn collect(directory: &Path, found: &mut Vec<PathBuf>) -> Result<(), MapperError> {
    let read = |source| MapperError::Read {
        path: directory.to_path_buf(),
        source,
    };
    for entry in std::fs::read_dir(directory).map_err(read)? {
        let path = entry.map_err(read)?.path();
        if path.is_dir() {
            collect(&path, found)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "yml" || extension == "yaml")
        {
            found.push(path);
        }
    }
    Ok(())
}

/// Reads and validates every OMOCL file below `directory`.
///
/// # Errors
///
/// Returns [`MapperError::Read`] when the directory cannot be read,
/// [`MapperError::Empty`] when it holds no file, and [`MapperError::Mappings`]
/// with every diagnostic when a file does not load.
pub fn read_set(directory: &Path) -> Result<MappingSet, MapperError> {
    let mut files = Vec::new();
    collect(directory, &mut files)?;
    if files.is_empty() {
        return Err(MapperError::Empty {
            path: directory.to_path_buf(),
        });
    }
    omocl::model::load::load_set(files, &FirstPartyConverters)
        .map_err(|diagnostics| MapperError::Mappings { diagnostics })
}

/// The OMOP vocabulary of the CDM database, as the engine asks it.
#[derive(Debug, Clone)]
pub struct CdmVocabulary {
    resolver: ConceptResolver,
}

impl CdmVocabulary {
    /// Asks `resolver`.
    #[must_use]
    pub fn new(resolver: ConceptResolver) -> Self {
        Self { resolver }
    }
}

/// Returns the lookup error of a failed operator or domain query.
fn lookup(error: omop_cdm::vocabulary::LookupError) -> LookupError {
    LookupError {
        what: error.to_string(),
        source: Box::new(error),
    }
}

impl ConceptSource for CdmVocabulary {
    fn resolve(
        &self,
        key: &SourceKey,
        date: &CdmDate,
    ) -> impl Future<Output = Result<Resolution, ResolveError>> + Send {
        self.resolver.resolve(key, date)
    }

    fn operator(
        &self,
        operator: Operator,
        date: &CdmDate,
    ) -> impl Future<Output = Result<Option<ConceptId>, LookupError>> + Send {
        let resolver = self.resolver.clone();
        let date = date.clone();
        async move {
            resolver
                .operator_concept(operator.symbol(), &date)
                .await
                .map(|id| id.map(|id| ConceptId::new(id.get())))
                .map_err(lookup)
        }
    }

    fn domain_concept(
        &self,
        domain: &str,
    ) -> impl Future<Output = Result<Option<ConceptId>, LookupError>> + Send {
        let resolver = self.resolver.clone();
        let domain = domain.to_owned();
        async move {
            resolver
                .domain_concept(&domain)
                .await
                .map(|id| id.map(|id| ConceptId::new(id.get())))
                .map_err(lookup)
        }
    }
}

/// The mapper over the loaded OMOCL set.
#[derive(Debug)]
pub struct OmoclMapper<S> {
    set: MappingSet,
    programs: Mutex<BTreeMap<String, Arc<Program>>>,
    vocabulary: S,
    aliases: VocabularyAliases,
}

impl<S: ConceptSource> OmoclMapper<S> {
    /// Maps with `set`, resolving concepts through `vocabulary`.
    #[must_use]
    pub fn new(set: MappingSet, vocabulary: S, aliases: VocabularyAliases) -> Self {
        Self {
            set,
            programs: Mutex::new(BTreeMap::new()),
            vocabulary,
            aliases,
        }
    }

    /// Returns the program of `template`, compiling it on first use.
    fn program(&self, template: &WebTemplateIndex) -> Result<Arc<Program>, Refusal> {
        let mut programs = self
            .programs
            .lock()
            .map_err(|_poisoned| Refusal::new("the compiled OMOCL programs are poisoned"))?;
        if let Some(program) = programs.get(template.template_id()) {
            return Ok(Arc::clone(program));
        }
        let program = omocl::resolve::compile(&self.set, template, &FirstPartyConverters).map_err(
            |errors| {
                Refusal::new(format!(
                    "the OMOCL mappings do not compile against `{}`: {}",
                    template.template_id(),
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<String>>()
                        .join("; ")
                ))
            },
        )?;
        programs.insert(template.template_id().to_owned(), Arc::clone(&program));
        Ok(program)
    }

    /// Maps one composition.
    async fn graph(&self, composition: &SourceComposition<'_>) -> Result<RecordGraph, Refusal> {
        let program = self.program(composition.template)?;
        let seams = Seams {
            concepts: &self.vocabulary,
            type_concept: ConceptId::new(composition.context.type_concept_id),
            aliases: &self.aliases,
        };
        let outcome = omocl::engine::run(
            &program,
            composition.composition,
            composition.template,
            &composition.source,
            composition.visit,
            &seams,
        )
        .await
        .map_err(|error| Refusal::new(crate::chain(&error)))?;
        Ok(outcome.into_parts().0)
    }
}

impl<S: ConceptSource> Mapper for OmoclMapper<S> {
    fn map(
        &self,
        composition: &SourceComposition<'_>,
    ) -> impl Future<Output = Result<RecordGraph, Refusal>> {
        self.graph(composition)
    }
}
