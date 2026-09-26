// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The entry point that loads FHIRconnect mapping files.
//!
//! One file goes through the YAML loader, the strict schema and the lowering;
//! a set goes through all three per file and then through the semantic rules,
//! which are the only ones that see more than one file. Every entry point
//! returns the whole diagnostic list rather than the first refusal, so one run
//! reports everything wrong with the input.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::header::metadata::MappingType;
use openehr_mapping_core::loader::MappingDocument;
use openehr_mapping_core::loader::load_file;

use crate::model::ast::ContextMappingFile;
use crate::model::ast::ModelMappingFile;
use crate::model::error::ModelCode;
use crate::model::parse::lower_context;
use crate::model::parse::lower_model;
use crate::model::schema::strict;
use crate::model::semantic::MappingCodeRegistry;
use crate::model::semantic::validate;

/// Every FHIRconnect file loaded together, indexed by `metadata.name`.
///
/// The header page calls `metadata.name` "a unique id used to identify the
/// mapping and reference it"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Metadata),
/// so a name is claimed by exactly one file across the whole set, whatever its
/// `type`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MappingSet {
    models: BTreeMap<MappingName, ModelMappingFile>,
    contexts: BTreeMap<MappingName, ContextMappingFile>,
}

impl MappingSet {
    /// Creates an empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one model or extension mapping file.
    ///
    /// # Errors
    ///
    /// Returns a diagnostic naming both files when the name is already
    /// claimed.
    pub fn insert_model(&mut self, file: ModelMappingFile) -> Result<(), Box<Diagnostic>> {
        let name = file.header().name().value().clone();
        if let Some(first) = self.file_of(&name) {
            return Err(Box::new(duplicate(
                &name,
                first,
                file.file(),
                file.header(),
            )));
        }
        self.models.insert(name, file);
        Ok(())
    }

    /// Adds one context mapping file.
    ///
    /// # Errors
    ///
    /// Returns a diagnostic naming both files when the name is already
    /// claimed.
    pub fn insert_context(&mut self, file: ContextMappingFile) -> Result<(), Box<Diagnostic>> {
        let name = file.header().name().value().clone();
        if let Some(first) = self.file_of(&name) {
            return Err(Box::new(duplicate(
                &name,
                first,
                file.file(),
                file.header(),
            )));
        }
        self.contexts.insert(name, file);
        Ok(())
    }

    /// Returns the model or extension mapping declaring `name`.
    #[must_use]
    pub fn model(&self, name: &MappingName) -> Option<&ModelMappingFile> {
        self.models.get(name)
    }

    /// Returns the context mapping declaring `name`.
    #[must_use]
    pub fn context(&self, name: &MappingName) -> Option<&ContextMappingFile> {
        self.contexts.get(name)
    }

    /// Returns every model and extension mapping, in mapping-name order.
    pub fn models(&self) -> impl Iterator<Item = &ModelMappingFile> {
        self.models.values()
    }

    /// Returns every context mapping, in mapping-name order.
    pub fn contexts(&self) -> impl Iterator<Item = &ContextMappingFile> {
        self.contexts.values()
    }

    /// Returns the file that claimed `name`.
    #[must_use]
    pub fn file_of(&self, name: &MappingName) -> Option<&Path> {
        self.models
            .get(name)
            .map(ModelMappingFile::file)
            .or_else(|| self.contexts.get(name).map(ContextMappingFile::file))
    }

    /// Returns how many files are loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.models.len() + self.contexts.len()
    }

    /// Whether no file is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.models.is_empty() && self.contexts.is_empty()
    }
}

/// Builds the diagnostic that refuses a second file claiming one name.
fn duplicate(
    name: &MappingName,
    first: &Path,
    second: &Path,
    header: &openehr_mapping_core::header::Header,
) -> Diagnostic {
    Diagnostic::error(
        second.to_path_buf(),
        DiagnosticCode::DuplicateMappingName,
        format!(
            "the mapping name `{name}` is declared by both {} and {}",
            first.display(),
            second.display()
        ),
    )
    .with_position(header.name().position())
    .with_mapping_name(name.clone())
    .with_model_path(ModelPath::root().field("metadata").field("name"))
}

/// Loads and validates one model or extension mapping file.
///
/// # Errors
///
/// Returns every refusal the loader, the strict schema and the lowering
/// raised, in that order.
pub fn load_model_file(file: impl AsRef<Path>) -> Result<ModelMappingFile, Vec<Diagnostic>> {
    let document = read(file.as_ref())?;
    check_schema(&document)?;
    lower_model(&document)
}

/// Loads and validates one context mapping file.
///
/// # Errors
///
/// Returns every refusal the loader, the strict schema and the lowering
/// raised, in that order.
pub fn load_context_file(file: impl AsRef<Path>) -> Result<ContextMappingFile, Vec<Diagnostic>> {
    let document = read(file.as_ref())?;
    check_schema(&document)?;
    lower_context(&document)
}

/// Loads and validates a whole set of mapping files.
///
/// Each file is read, schema-checked and lowered on its own, and the set is
/// then checked against the semantic rules, which need every file present.
/// The header's `type` decides which entry point a file takes.
///
/// # Errors
///
/// Returns every refusal from every file, followed by every semantic refusal.
/// A file that fails on its own is left out of the set, so a later rule about
/// it reports its absence rather than a second copy of the same failure.
pub fn load_set<I, P>(
    files: I,
    codes: &dyn MappingCodeRegistry,
) -> Result<MappingSet, Vec<Diagnostic>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut set = MappingSet::new();
    let mut diagnostics = Vec::new();
    let mut paths: Vec<PathBuf> = files
        .into_iter()
        .map(|file| file.as_ref().to_path_buf())
        .collect();
    paths.sort_unstable();

    for path in paths {
        let document = match read(&path) {
            Ok(document) => document,
            Err(mut errors) => {
                diagnostics.append(&mut errors);
                continue;
            }
        };
        if let Err(mut errors) = check_schema(&document) {
            diagnostics.append(&mut errors);
            continue;
        }
        let is_context =
            document.header().mapping_type().map(|t| *t.value()) == Some(MappingType::Context);
        let outcome = if is_context {
            lower_context(&document).and_then(|file| set.insert_context(file).map_err(|d| vec![*d]))
        } else {
            lower_model(&document).and_then(|file| set.insert_model(file).map_err(|d| vec![*d]))
        };
        if let Err(mut errors) = outcome {
            diagnostics.append(&mut errors);
        }
    }

    diagnostics.append(&mut validate(&set, codes));
    if diagnostics.is_empty() {
        Ok(set)
    } else {
        Err(diagnostics)
    }
}

/// Reads one file through the shared loader.
fn read(file: &Path) -> Result<MappingDocument, Vec<Diagnostic>> {
    load_file(file).map_err(|error| vec![Diagnostic::from(&error)])
}

/// Validates one loaded document against this crate's strict schemas.
fn check_schema(document: &MappingDocument) -> Result<(), Vec<Diagnostic>> {
    let schemas = strict::schemas().map_err(|error| {
        vec![Diagnostic::error(
            document.file().to_path_buf(),
            ModelCode::SchemaCompilation.into(),
            error.to_string(),
        )]
    })?;
    let diagnostics = schemas.validate_document(document);
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use openehr_mapping_core::diagnostic::DiagnosticCode;
    use openehr_mapping_core::loader::load_str;

    use super::MappingSet;
    use crate::model::parse::lower_model;
    use crate::model::semantic::StaticMappingCodes;
    use crate::model::semantic::validate;

    fn model(name: &str) -> String {
        format!(
            "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: {name}\n  version: \
             1.0.0\nspec:\n  system: FHIR\n  version: R4\n  openEhrConfig:\n    archetype: \
             openEHR-EHR-EVALUATION.test.v1\nmappings: []\n"
        )
    }

    #[test]
    fn a_duplicate_mapping_name_is_refused_naming_both_files() {
        let first = load_str("a.yml", &model("KDS_composition")).expect("a header");
        let second = load_str("b.yml", &model("KDS_composition")).expect("a header");
        let mut set = MappingSet::new();
        set.insert_model(lower_model(&first).expect("a model file"))
            .expect("the first file loads");
        let diagnostic = set
            .insert_model(lower_model(&second).expect("a model file"))
            .expect_err("the second is refused");
        assert_eq!(diagnostic.code(), &DiagnosticCode::DuplicateMappingName);
        assert!(diagnostic.message().contains("a.yml"), "{diagnostic}");
        assert!(diagnostic.message().contains("b.yml"), "{diagnostic}");
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
    }

    #[test]
    fn an_empty_set_validates() {
        let set = MappingSet::new();
        assert!(set.is_empty());
        assert!(validate(&set, &StaticMappingCodes::default()).is_empty());
    }
}
