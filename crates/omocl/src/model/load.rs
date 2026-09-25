// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The entry points that load OMOCL mapping files.
//!
//! One file goes through the shared YAML loader, the lowering, the authored
//! schema and the rules that hold within one file; a set goes through all of
//! that per file and then through the rules that need every file. Every entry
//! point returns the whole diagnostic list rather than the first refusal.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::header::ArchetypeId;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::loader::MappingDocument;
use openehr_mapping_core::registry::MappingRegistry;

use crate::model::ast::MappingFile;
use crate::model::error::ModelCode;
use crate::model::parse::lower;
use crate::model::schema::schema;
use crate::model::semantic::ConverterRegistry;
use crate::model::semantic::validate_file;
use crate::model::semantic::validate_set;

/// Every OMOCL file loaded together, by `metadata.name` and by archetype.
///
/// The shared registry of `openehr-mapping-core` holds the loaded documents
/// and refuses a second file claiming a name; this set holds the lowered file
/// beside each one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MappingSet {
    registry: MappingRegistry,
    files: BTreeMap<MappingName, MappingFile>,
}

impl MappingSet {
    /// Creates an empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one loaded document and its lowered file.
    ///
    /// # Errors
    ///
    /// Returns a diagnostic naming both files when the name is already
    /// claimed.
    pub fn insert(
        &mut self,
        document: MappingDocument,
        file: MappingFile,
    ) -> Result<(), Box<Diagnostic>> {
        let name = file.header.name().value().clone();
        self.registry
            .insert(document)
            .map_err(|error| Box::new(Diagnostic::from(&error)))?;
        self.files.insert(name, file);
        Ok(())
    }

    /// Returns the file declaring `name`.
    #[must_use]
    pub fn get(&self, name: &MappingName) -> Option<&MappingFile> {
        self.files.get(name)
    }

    /// Returns every file mapping `archetype`, in mapping-name order.
    pub fn for_archetype(&self, archetype: &ArchetypeId) -> impl Iterator<Item = &MappingFile> {
        self.registry
            .for_archetype(archetype)
            .filter_map(|document| self.files.get(document.header().name().value()))
    }

    /// Returns every file, in mapping-name order.
    pub fn files(&self) -> impl Iterator<Item = &MappingFile> {
        self.files.values()
    }

    /// Returns the shared registry of the loaded documents.
    #[must_use]
    pub const fn registry(&self) -> &MappingRegistry {
        &self.registry
    }

    /// Returns how many files are loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no file is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Loads and validates one OMOCL mapping file.
///
/// # Errors
///
/// Returns every refusal the loader, the lowering, the authored schema and
/// the rules of one file raised, in that order.
pub fn load_file(
    file: impl AsRef<Path>,
    converters: &dyn ConverterRegistry,
) -> Result<MappingFile, Vec<Diagnostic>> {
    check(&read(file.as_ref())?, converters)
}

/// Loads and validates one OMOCL mapping file from its text.
///
/// `file` names the source in every diagnostic.
///
/// # Errors
///
/// Returns whatever [`load_file`] returns.
pub fn load_str(
    file: impl Into<PathBuf>,
    source: &str,
    converters: &dyn ConverterRegistry,
) -> Result<MappingFile, Vec<Diagnostic>> {
    let document = openehr_mapping_core::loader::load_str(file, source)
        .map_err(|error| vec![Diagnostic::from(&error)])?;
    check(&document, converters)
}

/// Loads and validates a whole set of mapping files.
///
/// # Errors
///
/// Returns every refusal from every file, followed by every refusal of the
/// rules that need the set. A file that fails on its own is left out of the
/// set, so a rule about it reports its absence rather than a second copy of
/// the same failure.
pub fn load_set<I, P>(
    files: I,
    converters: &dyn ConverterRegistry,
) -> Result<MappingSet, Vec<Diagnostic>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut paths: Vec<PathBuf> = files
        .into_iter()
        .map(|file| file.as_ref().to_path_buf())
        .collect();
    paths.sort_unstable();

    let mut set = MappingSet::new();
    let mut diagnostics = Vec::new();
    for path in paths {
        let outcome = read(&path).and_then(|document| {
            let file = check(&document, converters)?;
            set.insert(document, file).map_err(|error| vec![*error])
        });
        if let Err(mut errors) = outcome {
            diagnostics.append(&mut errors);
        }
    }
    diagnostics.append(&mut validate_set(&set));
    if diagnostics.is_empty() {
        Ok(set)
    } else {
        Err(diagnostics)
    }
}

/// Reads one file through the shared loader.
fn read(file: &Path) -> Result<MappingDocument, Vec<Diagnostic>> {
    openehr_mapping_core::loader::load_file(file).map_err(|error| vec![Diagnostic::from(&error)])
}

/// Lowers one document, validates it against the schema and applies the rules
/// of one file.
fn check(
    document: &MappingDocument,
    converters: &dyn ConverterRegistry,
) -> Result<MappingFile, Vec<Diagnostic>> {
    let file = lower(document)?;
    let schema = schema().map_err(|error| {
        vec![Diagnostic::error(
            document.file().to_path_buf(),
            ModelCode::SchemaCompilation.into(),
            error.to_string(),
        )]
    })?;
    let mut diagnostics = schema.validate_document(document);
    diagnostics.append(&mut validate_file(&file, converters));
    if diagnostics.is_empty() {
        Ok(file)
    } else {
        Err(diagnostics)
    }
}
