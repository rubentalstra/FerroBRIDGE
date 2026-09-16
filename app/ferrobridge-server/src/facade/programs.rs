// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The loaded mapping set: one compiled program per context, ready to run.
//!
//! A mapping directory is read once at boot: every FHIRconnect file is loaded
//! and validated, the operational template each context names is fetched from
//! the CDR, and the context is compiled against it into an immutable program
//! (`docs/architecture.md` §4.3). A mapping that does not compile is a boot
//! refusal, never a failure on the request that first touches it.
//!
//! Selection is by the set of `meta.profile`. The engine chapter keys on
//! `meta.url`, which R4 does not define; the element is `meta.profile`, a list
//! (<https://hl7.org/fhir/R4/resource.html#Meta>), so matching is set
//! membership and every declared profile is considered
//! (`docs/architecture.md` §4.6).

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use fhirconnect::model::load::MappingSet;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::Program;
use fhirconnect::resolve::program::TemplateId;
use fhirconnect::resolve::select::SelectError;
use fhirconnect::resolve::select::select_by_profile_pinned;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::TemplateSource;

/// The file extensions a mapping directory is read for.
///
/// FHIRconnect writes YAML and states no extension, so both spellings are
/// read and nothing else is (no specification governs this: our own design).
const MAPPING_EXTENSIONS: [&str; 2] = ["yml", "yaml"];

/// Why the mapping set did not load.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// The mapping directory could not be read.
    #[error("the mapping directory {} could not be read", path.display())]
    Directory {
        /// The path that was tried.
        path: PathBuf,
        /// What the file system reported.
        #[source]
        source: std::io::Error,
    },
    /// A mapping file is not valid FHIRconnect.
    #[error("{} mapping diagnostics, the first of which is: {first}", count)]
    Mappings {
        /// How many diagnostics the load raised.
        count: usize,
        /// The first diagnostic, rendered.
        first: String,
    },
    /// A context names a template with no identifier.
    #[error("the context {context} names no template id")]
    NoTemplateId {
        /// The context that names none.
        context: String,
    },
    /// The CDR does not hold the template a context names.
    #[error("the CDR holds no template {template}, which the context {context} maps")]
    UnknownTemplate {
        /// The template that was asked for.
        template: String,
        /// The context that names it.
        context: String,
    },
    /// The template could not be fetched.
    #[error("the template {template} could not be fetched from the CDR")]
    Fetch {
        /// The template that was asked for.
        template: String,
        /// What the client reported.
        #[source]
        source: Box<ferrobridge_openehr::error::Error>,
    },
    /// The template does not build a Web Template.
    #[error("the template {template} does not build a Web Template")]
    Template {
        /// The template that refused.
        template: String,
        /// What the index builder reported.
        #[source]
        source: Box<openehr_mapping_core::template::PathError>,
    },
    /// The CDR refused the template fetch, or served a generation this
    /// version does not read.
    #[error("the CDR did not serve the template {template}: {detail}")]
    TemplateRefused {
        /// The template that was asked for.
        template: String,
        /// What the CDR answered, as the client reported it.
        detail: String,
    },
    /// A template identifier the CDR route cannot carry.
    #[error("{template} is no template identifier the CDR route can carry")]
    TemplateName {
        /// The identifier a context wrote.
        template: String,
        /// What the identifier's constructor reported.
        #[source]
        source: Box<ferrobridge_openehr::ids::IdError>,
    },
}

/// One compiled context, with the template index it runs against.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The compiled program.
    program: Arc<Program>,
    /// The index of the template the program was compiled against.
    index: Arc<WebTemplateIndex>,
}

impl Loaded {
    /// Returns the compiled program.
    #[must_use]
    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    /// Returns the index of the template the program runs against.
    #[must_use]
    pub fn index(&self) -> &Arc<WebTemplateIndex> {
        &self.index
    }
}

/// Every program this deployment loaded.
#[derive(Debug, Default)]
pub struct Programs {
    /// The programs, in context-name order.
    loaded: Vec<Loaded>,
    /// The programs alone, for the selection functions of `fhirconnect`.
    programs: Vec<Arc<Program>>,
}

impl Programs {
    /// Returns a registry over `loaded`, ordered by context name.
    #[must_use]
    pub fn new(mut loaded: Vec<Loaded>) -> Self {
        loaded.sort_by(|left, right| {
            left.program
                .context()
                .as_str()
                .cmp(right.program.context().as_str())
        });
        let programs = loaded
            .iter()
            .map(|entry| Arc::clone(&entry.program))
            .collect();
        Self { loaded, programs }
    }

    /// Returns how many programs were loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.loaded.len()
    }

    /// Whether no program was loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }

    /// Returns every loaded program, in context-name order.
    #[must_use]
    pub fn loaded(&self) -> &[Loaded] {
        &self.loaded
    }

    /// Returns the resource types at least one program maps, in order.
    ///
    /// The `CapabilityStatement` names exactly these, and a type outside the
    /// set answers `404` with `not-supported` on the wire
    /// (`docs/architecture.md` §4.6).
    #[must_use]
    pub fn resource_types(&self) -> BTreeSet<String> {
        self.loaded
            .iter()
            .map(|entry| String::from(entry.program.resource().as_str()))
            .collect()
    }

    /// Whether any program maps `resource_type`.
    #[must_use]
    pub fn supports(&self, resource_type: &str) -> bool {
        self.loaded
            .iter()
            .any(|entry| entry.program.resource().as_str() == resource_type)
    }

    /// Returns the program the claimed profiles select, pinned by `template`.
    ///
    /// # Errors
    ///
    /// Returns [`SelectError::NoMatch`] when no program claims one of the
    /// profiles, [`SelectError::Ambiguous`] when several do and `template`
    /// pins none, and [`SelectError::ProfileVersion`] when the only claimants
    /// pin another profile version.
    pub fn select(
        &self,
        claimed: &[String],
        template: Option<&TemplateId>,
    ) -> Result<&Loaded, SelectError> {
        let chosen = select_by_profile_pinned(&self.programs, claimed, template)?;
        self.loaded
            .iter()
            .find(|entry| Arc::ptr_eq(&entry.program, chosen))
            .ok_or_else(|| SelectError::NoMatch {
                wanted: format!("the profiles [{}]", claimed.join(", ")),
            })
    }

    /// Returns the program compiled from the context named `context`.
    #[must_use]
    pub fn by_context(&self, context: &str) -> Option<&Loaded> {
        self.loaded
            .iter()
            .find(|entry| entry.program.context().as_str() == context)
    }

    /// Returns every candidate context that claims one of `claimed`.
    ///
    /// An ambiguous selection reports these, so a caller learns which
    /// `templateId` would pin the choice.
    #[must_use]
    pub fn candidates(&self, claimed: &[String]) -> Vec<String> {
        self.loaded
            .iter()
            .filter(|entry| {
                claimed.iter().any(|entry_url| {
                    entry_url.split('|').next().unwrap_or(entry_url)
                        == entry.program.profile().url().as_str()
                })
            })
            .map(|entry| {
                format!(
                    "{} ({})",
                    entry.program.context(),
                    entry.program.template().id()
                )
            })
            .collect()
    }
}

/// Returns the FHIRconnect files under `directory`, sorted.
///
/// # Errors
///
/// Returns [`LoadError::Directory`] when the directory cannot be listed.
pub fn mapping_files(directory: &Path) -> Result<Vec<PathBuf>, LoadError> {
    let mut found = Vec::new();
    collect(directory, &mut found)?;
    found.sort();
    Ok(found)
}

/// Adds every mapping file under `directory` to `found`, recursively.
fn collect(directory: &Path, found: &mut Vec<PathBuf>) -> Result<(), LoadError> {
    let entries = std::fs::read_dir(directory).map_err(|source| LoadError::Directory {
        path: directory.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| LoadError::Directory {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found)?;
            continue;
        }
        let extension = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_ascii_lowercase);
        if extension.is_some_and(|found| MAPPING_EXTENSIONS.contains(&found.as_str())) {
            found.push(path);
        }
    }
    Ok(())
}

/// Reads and validates the mapping set under `directory`.
///
/// # Errors
///
/// Returns [`LoadError::Directory`] when the directory cannot be listed and
/// [`LoadError::Mappings`] when a file is not valid FHIRconnect.
pub fn read_set(directory: &Path) -> Result<MappingSet, LoadError> {
    let files = mapping_files(directory)?;
    fhirconnect::model::load::load_set(files, &StaticMappingCodes::default())
        .map_err(|diagnostics| mappings_error(&diagnostics))
}

/// Compiles every context of `set` against the templates in `indexes`.
///
/// `indexes` is keyed by template identifier; a context whose template is
/// absent is a refusal, because "a mapping cannot name a template the CDR has
/// not loaded" (`docs/architecture.md` §12).
///
/// # Errors
///
/// Returns [`LoadError::NoTemplateId`] for a context naming no template,
/// [`LoadError::UnknownTemplate`] for one whose template is absent, and
/// [`LoadError::Mappings`] for a context that does not compile.
pub fn compile_set(
    set: &MappingSet,
    indexes: &BTreeMap<String, Arc<WebTemplateIndex>>,
) -> Result<Programs, LoadError> {
    let mut loaded = Vec::new();
    for context in set.contexts() {
        let name = context.header().name().value().clone();
        let template = template_of(context)?;
        let Some(index) = indexes.get(&template) else {
            return Err(LoadError::UnknownTemplate {
                template,
                context: name.to_string(),
            });
        };
        let program = compile(
            set,
            &name,
            index,
            &fhir_types::r4::schema::SCHEMAS,
            &StaticMappingCodes::default(),
        )
        .map_err(|diagnostics| mappings_error(&diagnostics))?;
        loaded.push(Loaded {
            program,
            index: Arc::clone(index),
        });
    }
    Ok(Programs::new(loaded))
}

/// Returns the template identifier one context names.
fn template_of(context: &fhirconnect::model::ast::ContextMappingFile) -> Result<String, LoadError> {
    context
        .context()
        .template
        .id
        .as_ref()
        .map(|located| located.value().clone())
        .ok_or_else(|| LoadError::NoTemplateId {
            context: context.file().display().to_string(),
        })
}

/// Returns the templates every context of `set` names, fetched from the CDR.
///
/// # Errors
///
/// Returns [`LoadError::TemplateName`] for an identifier the CDR route cannot
/// carry, [`LoadError::Fetch`] when the call does not reach a documented
/// answer, [`LoadError::UnknownTemplate`] when the CDR holds no such template,
/// and [`LoadError::Template`] when the template does not build a Web
/// Template.
pub async fn fetch_templates(
    set: &MappingSet,
    client: &ferrobridge_openehr::client::Client,
) -> Result<BTreeMap<String, Arc<WebTemplateIndex>>, LoadError> {
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for context in set.contexts() {
        wanted.insert(template_of(context)?);
    }
    let mut indexes = BTreeMap::new();
    for template in wanted {
        let id = ferrobridge_openehr::ids::TemplateId::new(&template).map_err(|source| {
            LoadError::TemplateName {
                template: template.clone(),
                source: Box::new(source),
            }
        })?;
        let outcome = client
            .template(&id)
            .await
            .map_err(|source| LoadError::Fetch {
                template: template.clone(),
                source: Box::new(source),
            })?;
        let source = match outcome {
            ferrobridge_openehr::template::TemplateOutcome::Found(source) => source,
            ferrobridge_openehr::template::TemplateOutcome::UnknownTemplate => {
                return Err(LoadError::UnknownTemplate {
                    template,
                    context: String::from("a loaded context"),
                });
            }
            ferrobridge_openehr::template::TemplateOutcome::BadRequest(upstream) => {
                return Err(LoadError::TemplateRefused {
                    template,
                    detail: upstream.to_string(),
                });
            }
            other => {
                return Err(LoadError::TemplateRefused {
                    template,
                    detail: format!(
                        "the client answered an outcome this version does not read ({other:?})"
                    ),
                });
            }
        };
        let index = build_index(&template, source)?;
        indexes.insert(template, Arc::new(index));
    }
    Ok(indexes)
}

/// Returns the Web Template index of one fetched template.
fn build_index(
    template: &str,
    source: ferrobridge_openehr::template::TemplateSource,
) -> Result<WebTemplateIndex, LoadError> {
    let source = match source {
        ferrobridge_openehr::template::TemplateSource::Opt14(opt) => TemplateSource::Opt14(opt),
        ferrobridge_openehr::template::TemplateSource::Opt2 {
            template: opt,
            resolved_id,
        } => TemplateSource::Opt2 {
            template: opt,
            resolved_id: resolved_id.as_str().to_owned(),
        },
        other => {
            return Err(LoadError::TemplateRefused {
                template: template.to_owned(),
                detail: format!(
                    "the CDR served a generation this version does not read ({other:?})"
                ),
            });
        }
    };
    WebTemplateIndex::build(&source).map_err(|source| LoadError::Template {
        template: template.to_owned(),
        source: Box::new(source),
    })
}

/// Returns the refusal a diagnostic list describes.
fn mappings_error(diagnostics: &[Diagnostic]) -> LoadError {
    let first = diagnostics.first().map_or_else(
        || String::from("no diagnostic was reported"),
        |diagnostic| {
            format!(
                "{} {}: {}",
                diagnostic.file().display(),
                diagnostic.code(),
                diagnostic.message()
            )
        },
    );
    LoadError::Mappings {
        count: diagnostics.len(),
        first,
    }
}

#[cfg(test)]
mod tests {
    use super::{Programs, mapping_files};

    #[test]
    fn an_empty_registry_supports_no_type() {
        let programs = Programs::default();
        assert!(programs.is_empty());
        assert_eq!(0, programs.len());
        assert!(!programs.supports("Condition"));
        assert!(programs.resource_types().is_empty());
    }

    #[test]
    fn only_yaml_files_are_read_and_the_order_is_stable() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        for name in ["b.yml", "a.yaml", "notes.txt", "c.YML"] {
            std::fs::write(directory.path().join(name), "x: 1\n").expect("a fixture file");
        }
        std::fs::create_dir_all(directory.path().join("nested")).expect("a nested directory");
        std::fs::write(directory.path().join("nested/d.yml"), "x: 1\n").expect("a fixture file");
        let found = mapping_files(directory.path()).expect("the directory lists");
        let names: Vec<String> = found
            .iter()
            .filter_map(|path| path.file_name().and_then(std::ffi::OsStr::to_str))
            .map(str::to_owned)
            .collect();
        assert_eq!(
            vec![
                String::from("a.yaml"),
                String::from("b.yml"),
                String::from("c.YML"),
                String::from("d.yml"),
            ],
            names
        );
    }

    #[test]
    fn a_directory_that_does_not_exist_is_a_refusal() {
        let error = mapping_files(std::path::Path::new("/ferrobridge/no/such/directory"))
            .expect_err("an absent directory is refused");
        assert!(format!("{error}").contains("could not be read"), "{error}");
    }
}
