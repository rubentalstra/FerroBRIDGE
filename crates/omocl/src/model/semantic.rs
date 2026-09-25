// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rules of an OMOCL mapping set that a schema cannot express.
//!
//! Three hold within one file: two keys of one record may not write the same
//! CDM column, and a `CustomMapping` names a registered converter. One needs
//! the set: an `Include` names an archetype some loaded file maps. The domain
//! check of a literal concept id needs a vocabulary and runs apart from the
//! others, through [`check_concept_domains`], once one is loaded.

use core::fmt;
use std::collections::BTreeMap;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::position::Position;

use crate::model::ast::Alternative;
use crate::model::ast::ConceptId;
use crate::model::ast::DomainId;
use crate::model::ast::Entity;
use crate::model::ast::MappingFile;
use crate::model::ast::Record;
use crate::model::error::ModelCode;
use crate::model::load::MappingSet;
use crate::model::projection::Part;

/// The converters a `CustomMapping` may name.
pub trait ConverterRegistry: fmt::Debug {
    /// Whether a converter of this name is registered.
    fn contains(&self, name: &str) -> bool;
}

/// The converters FerroBRIDGE ships.
///
/// The one converter the OMOCL library names is
/// `FactRelationshipCustomConverter`
/// (`docs/specs/omocl/medical_data/observation/Laboratory_test_result_v1.yml`),
/// and no grammar artefact names `CustomMapping` at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FirstPartyConverters;

impl FirstPartyConverters {
    /// The names of the first-party converters.
    pub const NAMES: &'static [&'static str] = &["FactRelationshipCustomConverter"];
}

impl ConverterRegistry for FirstPartyConverters {
    fn contains(&self, name: &str) -> bool {
        Self::NAMES.contains(&name)
    }
}

/// Applies the rules that hold within one file.
///
/// The returned list is empty when the file is valid.
#[must_use]
pub fn validate_file(file: &MappingFile, converters: &dyn ConverterRegistry) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (index, entity) in file.entities.iter().enumerate() {
        let path = ModelPath::root().field("mappings").index(index);
        match *entity {
            Entity::Record(ref record) => claimed_twice(file, record, &path, &mut diagnostics),
            Entity::CustomMapping(ref custom) => {
                if !converters.contains(custom.name.value()) {
                    diagnostics.push(error(
                        file,
                        ModelCode::UnknownConverter,
                        custom.name.position(),
                        path.field("name"),
                        format!(
                            "the `CustomMapping` `{}` names no registered converter",
                            custom.name.value()
                        ),
                    ));
                }
            }
            Entity::Include(_) => {}
        }
    }
    diagnostics
}

/// Applies the rules that need every file of the set.
///
/// The returned list is empty when the set is valid.
#[must_use]
pub fn validate_set(set: &MappingSet) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for file in set.files() {
        for (index, entity) in file.entities.iter().enumerate() {
            let Entity::Include(ref include) = *entity else {
                continue;
            };
            let archetype = include.archetype_id.value();
            if set.for_archetype(archetype).next().is_none() {
                diagnostics.push(error(
                    file,
                    ModelCode::UnresolvedInclude,
                    include.archetype_id.position(),
                    ModelPath::root()
                        .field("mappings")
                        .index(index)
                        .field("archetype_id"),
                    format!("the `Include` names `{archetype}`, which no loaded file maps"),
                ));
            }
        }
    }
    diagnostics
}

/// Refuses two keys of one record that write the same CDM column.
fn claimed_twice(
    file: &MappingFile,
    record: &Record,
    path: &ModelPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut claimed: BTreeMap<&'static str, &'static str> = BTreeMap::new();
    for column in &record.columns {
        for projected in column.projection.columns {
            if let Some(first) = claimed.insert(projected.column, column.key()) {
                diagnostics.push(error(
                    file,
                    ModelCode::ColumnClaimedTwice,
                    column.key_position,
                    path.field(column.key()),
                    format!(
                        "`{first}` and `{}` both write `{}.{}`",
                        column.key(),
                        record.target.value().table(),
                        projected.column
                    ),
                ));
            }
        }
    }
}

/// Checks every literal concept id of the set against a loaded vocabulary.
///
/// `domain_of` answers the `domain_id` of a concept, or `None` when the
/// vocabulary holds no such concept. A literal is checked where its key
/// writes a concept column: the literal must be a concept of the vocabulary,
/// and when the CDM field definitions name a domain for that column
/// (`fkDomain`, carried as `omop_cdm::meta::ColumnMeta::fk_domain`) its
/// domain must be that one. The CDM routes a record by the domain of its
/// standard concept ("Write the data record into the table(s) corresponding
/// to the domain of the Standard `CONCEPT_ID`(s)",
/// <https://ohdsi.github.io/CommonDataModel/dataModelConventions.html>).
/// Concept `0` is exempt: the CDM writes it where no standard concept exists.
///
/// The returned list is empty when every literal agrees.
#[must_use]
pub fn check_concept_domains(
    set: &MappingSet,
    domain_of: &dyn Fn(ConceptId) -> Option<DomainId>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for file in set.files() {
        for (index, entity) in file.entities.iter().enumerate() {
            let Entity::Record(ref record) = *entity else {
                continue;
            };
            let path = ModelPath::root().field("mappings").index(index);
            check_record(file, record, &path, domain_of, &mut diagnostics);
        }
    }
    diagnostics
}

/// Checks the literal concept ids of one record.
fn check_record(
    file: &MappingFile,
    record: &Record,
    path: &ModelPath,
    domain_of: &dyn Fn(ConceptId) -> Option<DomainId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let target = *record.target.value();
    let table = omop_cdm::meta::table(target.table());
    for column in &record.columns {
        let Some(concept_column) = column.projection.columns.iter().find(|projected| {
            matches!(projected.part, Part::Concept | Part::Units | Part::Operator)
        }) else {
            continue;
        };
        let expected = table
            .and_then(|table| table.column(concept_column.column))
            .and_then(|meta| meta.fk_domain);
        let column_path = path.field(column.key()).field("alternatives");
        for (position, literal, literal_path) in literals(&column.alternatives, &column_path) {
            if literal == ConceptId::NO_MATCHING_CONCEPT {
                continue;
            }
            let Some(domain) = domain_of(literal) else {
                diagnostics.push(error(
                    file,
                    ModelCode::UnknownConcept,
                    position,
                    literal_path,
                    format!("the concept `{literal}` is not in the loaded vocabulary"),
                ));
                continue;
            };
            if let Some(expected) = expected
                && domain.as_str() != expected
            {
                diagnostics.push(error(
                    file,
                    ModelCode::DomainMismatch,
                    position,
                    literal_path,
                    format!(
                        "the concept `{literal}` is in the `{domain}` domain; `{}.{}` takes the \
                         `{expected}` domain",
                        target.table(),
                        concept_column.column
                    ),
                ));
            }
        }
    }
}

/// Returns every literal concept id of an `alternatives` list, positioned.
fn literals(
    alternatives: &[Alternative],
    path: &ModelPath,
) -> Vec<(Position, ConceptId, ModelPath)> {
    let mut found = Vec::new();
    for (index, alternative) in alternatives.iter().enumerate() {
        let item = path.index(index);
        match *alternative {
            Alternative::Code(ref code) => {
                found.push((code.position(), *code.value(), item.field("code")));
            }
            Alternative::ConceptMap(ref map) => {
                for (at_code, concept) in &map.mapping {
                    found.push((
                        concept.position(),
                        *concept.value(),
                        item.field("conceptMap")
                            .field("mapping")
                            .field(at_code.as_str()),
                    ));
                }
            }
            Alternative::Path(_) | Alternative::Multiplication(_) => {}
        }
    }
    found
}

/// Builds one positioned refusal about `file`.
fn error(
    file: &MappingFile,
    code: ModelCode,
    position: Position,
    path: ModelPath,
    message: String,
) -> Diagnostic {
    Diagnostic::error(file.file.clone(), code.into(), message)
        .with_position(position)
        .with_model_path(path)
        .with_mapping_name(file.header.name().value().clone())
}

#[cfg(test)]
mod tests {
    use super::ConverterRegistry;
    use super::FirstPartyConverters;

    #[test]
    fn the_first_party_registry_holds_the_one_library_converter() {
        assert!(FirstPartyConverters.contains("FactRelationshipCustomConverter"));
        assert!(!FirstPartyConverters.contains("factRelationshipCustomConverter"));
    }
}
