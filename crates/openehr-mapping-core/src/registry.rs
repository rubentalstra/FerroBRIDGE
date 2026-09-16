// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The registry of loaded mapping files.
//!
//! The FHIRconnect header page calls `metadata.name` "a unique id used to
//! identify the mapping and reference it"
//! (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Metadata),
//! and every cross-file reference the language defines resolves by that name,
//! so a second file declaring a name already loaded is a load error naming
//! both files. An archetype is the other way round: a model mapping and its
//! extensions all name the same archetype, so the archetype index holds a set.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticCode;
use crate::diagnostic::ModelPath;
use crate::header::ArchetypeId;
use crate::header::MappingName;
use crate::loader::MappingDocument;
use crate::position::Position;

/// Why a mapping file could not join a registry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// Two files declare the same `metadata.name`.
    #[error(
        "the mapping name `{name}` is declared by both {} and {}",
        first_file.display(),
        second_file.display()
    )]
    DuplicateName {
        /// The name both files declare.
        name: MappingName,
        /// The file that claimed the name first.
        first_file: PathBuf,
        /// The file refused for declaring it again.
        second_file: PathBuf,
        /// Where the refused file writes the name.
        position: Position,
    },
}

impl RegistryError {
    /// Returns the code this refusal reports.
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        match *self {
            Self::DuplicateName { .. } => DiagnosticCode::DuplicateMappingName,
        }
    }
}

impl From<&RegistryError> for Diagnostic {
    fn from(error: &RegistryError) -> Self {
        match *error {
            RegistryError::DuplicateName {
                ref name,
                ref second_file,
                position,
                ..
            } => Self::error(second_file.clone(), error.code(), error.to_string())
                .with_position(position)
                .with_mapping_name(name.clone())
                .with_model_path(ModelPath::root().field("metadata").field("name")),
        }
    }
}

/// Every loaded mapping file, indexed by name and by archetype.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MappingRegistry {
    by_name: BTreeMap<MappingName, MappingDocument>,
    by_archetype: BTreeMap<ArchetypeId, BTreeSet<MappingName>>,
}

impl MappingRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one loaded document.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DuplicateName`] when another file already
    /// declared this document's `metadata.name`.
    pub fn insert(&mut self, document: MappingDocument) -> Result<(), RegistryError> {
        let name = document.header().name().value().clone();
        if let Some(first) = self.by_name.get(&name) {
            return Err(RegistryError::DuplicateName {
                name,
                first_file: first.file().to_path_buf(),
                second_file: document.file().to_path_buf(),
                position: document.header().name().position(),
            });
        }
        if let Some(archetype) = document.header().archetype() {
            self.by_archetype
                .entry(archetype.value().clone())
                .or_default()
                .insert(name.clone());
        }
        self.by_name.insert(name, document);
        Ok(())
    }

    /// Returns the document declaring `name`.
    #[must_use]
    pub fn get(&self, name: &MappingName) -> Option<&MappingDocument> {
        self.by_name.get(name)
    }

    /// Returns every document whose header names `archetype`, in name order.
    pub fn for_archetype(
        &self,
        archetype: &ArchetypeId,
    ) -> impl Iterator<Item = &MappingDocument> + '_ {
        self.by_archetype
            .get(archetype)
            .into_iter()
            .flatten()
            .filter_map(|name| self.by_name.get(name))
    }

    /// Returns the file declaring `name`.
    #[must_use]
    pub fn file_of(&self, name: &MappingName) -> Option<&Path> {
        self.by_name.get(name).map(MappingDocument::file)
    }

    /// Returns every loaded document, in mapping-name order.
    pub fn documents(&self) -> impl Iterator<Item = &MappingDocument> + '_ {
        self.by_name.values()
    }

    /// Returns every loaded mapping name, in order.
    pub fn names(&self) -> impl Iterator<Item = &MappingName> + '_ {
        self.by_name.keys()
    }

    /// Returns how many documents are loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether no document is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use super::MappingRegistry;
    use super::RegistryError;
    use crate::diagnostic::Diagnostic;
    use crate::diagnostic::DiagnosticCode;
    use crate::header::ArchetypeId;
    use crate::header::MappingName;
    use crate::loader::load_str;

    fn source(name: &str, archetype: &str) -> String {
        format!(
            "grammar: OMOCL/v1.0.0\ntype: model\nmetadata:\n  name: {name}\n  version: 1.0.0\n\
             spec:\n  system: OMOP\n  version: 5.4\n  openEhrConfig:\n    archetype: {archetype}\n"
        )
    }

    #[test]
    fn a_duplicate_name_is_refused_naming_both_files() {
        let first = load_str("a.yml", &source("shared", "openEHR-EHR-CLUSTER.device.v1"))
            .expect("a well-formed header");
        let second = load_str("b.yml", &source("shared", "openEHR-EHR-CLUSTER.device.v1"))
            .expect("a well-formed header");
        let mut registry = MappingRegistry::new();
        registry.insert(first).expect("the first file loads");
        let error = registry.insert(second).expect_err("the second is refused");
        assert!(matches!(error, RegistryError::DuplicateName { .. }));
        let rendered = error.to_string();
        assert!(rendered.contains("a.yml"), "{rendered}");
        assert!(rendered.contains("b.yml"), "{rendered}");
        assert_eq!(
            Diagnostic::from(&error).code(),
            &DiagnosticCode::DuplicateMappingName
        );
    }

    #[test]
    fn several_mappings_may_share_one_archetype() {
        let archetype = "openEHR-EHR-OBSERVATION.body_weight.v2";
        let mut registry = MappingRegistry::new();
        registry
            .insert(load_str("model.yml", &source("model", archetype)).expect("a header"))
            .expect("the model loads");
        registry
            .insert(load_str("ext.yml", &source("extension", archetype)).expect("a header"))
            .expect("the extension loads");
        let id = ArchetypeId::from_str(archetype).expect("a well-formed archetype id");
        let names: Vec<&str> = registry
            .for_archetype(&id)
            .map(|document| document.header().name().value().as_str())
            .collect();
        assert_eq!(names, vec!["extension", "model"]);
        assert_eq!(registry.len(), 2);
        let model = MappingName::new("model").expect("a well-formed name");
        assert_eq!(
            registry.file_of(&model).map(std::path::Path::to_path_buf),
            Some(std::path::PathBuf::from("model.yml"))
        );
    }
}
