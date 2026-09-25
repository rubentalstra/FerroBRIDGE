// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two laboratory files the first OMOP round trip loads, lowered and
//! pinned as `insta` snapshots of their file model.

use std::error::Error;
use std::fmt::Write as _;

use omocl::model::ast::Alternative;
use omocl::model::ast::Column;
use omocl::model::ast::Entity;
use omocl::model::ast::Factor;
use omocl::model::ast::MappingFile;
use omocl::model::load::load_file;
use omocl::model::semantic::FirstPartyConverters;
use openehr_mapping_core::position::Position;

use crate::support::corpus_path;

/// Renders a lowered file one construct per line, with positions.
fn render(file: &MappingFile) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    let header = file.header();
    writeln!(
        out,
        "{} {} archetype={} @{}",
        header.name().value(),
        header.version().value(),
        file.archetype.value(),
        file.archetype.position()
    )?;
    writeln!(
        out,
        "spec {:?} {:?} @{}",
        file.spec.system.value(),
        file.spec.version.value(),
        file.spec.position
    )?;
    for entity in &file.entities {
        match *entity {
            Entity::Record(ref record) => {
                writeln!(out, "record {} @{}", record.target.value(), record.position)?;
                if let Some(ref base) = record.base_path {
                    writeln!(out, "  base_path {} @{}", base.value(), base.position())?;
                }
                for column in &record.columns {
                    writeln!(
                        out,
                        "  {} optional={} @{}",
                        column.key(),
                        column.is_optional(),
                        column.key_position
                    )?;
                    for alternative in &column.alternatives {
                        render_alternative(&mut out, alternative)?;
                    }
                }
            }
            Entity::Include(ref include) => {
                writeln!(
                    out,
                    "include {} @{}",
                    include.archetype_id.value(),
                    include.position
                )?;
                if let Some(ref base) = include.base_path {
                    writeln!(out, "  base_path {} @{}", base.value(), base.position())?;
                }
            }
            Entity::CustomMapping(ref custom) => {
                writeln!(
                    out,
                    "custom {} @{}",
                    custom.name.value(),
                    custom.name.position()
                )?;
            }
            _ => writeln!(out, "entity @{}", entity.position())?,
        }
    }
    Ok(out)
}

/// Renders one alternative.
fn render_alternative(out: &mut String, alternative: &Alternative) -> std::fmt::Result {
    let at = |position: Position| position.to_string();
    match *alternative {
        Alternative::Path(ref path) => {
            writeln!(out, "    path {} @{}", path.value(), at(path.position()))
        }
        Alternative::Code(ref code) => {
            writeln!(out, "    code {} @{}", code.value(), at(code.position()))
        }
        Alternative::ConceptMap(ref map) => {
            writeln!(
                out,
                "    conceptMap {} @{}",
                map.path.value(),
                at(map.position)
            )?;
            for (code, concept) in &map.mapping {
                writeln!(out, "      {code} -> {}", concept.value())?;
            }
            Ok(())
        }
        Alternative::Multiplication(ref product) => {
            writeln!(out, "    multiplication @{}", at(product.position))?;
            for factor in &product.factors {
                match *factor {
                    Factor::Path(ref path) => writeln!(out, "      path {}", path.value())?,
                    Factor::Code(ref code) => writeln!(out, "      code {}", code.value())?,
                    _ => writeln!(out, "      factor")?,
                }
            }
            Ok(())
        }
        _ => writeln!(out, "    alternative"),
    }
}

#[test]
fn the_laboratory_result_file_lowers_to_its_snapshot() -> Result<(), Box<dyn Error>> {
    let file = load_file(
        corpus_path("medical_data/observation/Laboratory_test_result_v1.yml"),
        &FirstPartyConverters,
    )
    .map_err(|diagnostics| crate::support::render(&diagnostics))?;
    insta::assert_snapshot!("laboratory_test_result_v1", render(&file)?);
    Ok(())
}

#[test]
fn the_laboratory_analyte_file_lowers_to_its_snapshot() -> Result<(), Box<dyn Error>> {
    let file = load_file(
        corpus_path("medical_data/cluster/Laboratory_test_analyte_v1.yml"),
        &FirstPartyConverters,
    )
    .map_err(|diagnostics| crate::support::render(&diagnostics))?;
    insta::assert_snapshot!("laboratory_test_analyte_v1", render(&file)?);
    Ok(())
}

#[test]
fn an_aliased_alternatives_list_is_lowered_once_per_alias() -> Result<(), Box<dyn Error>> {
    let file = load_file(
        corpus_path("medical_data/cluster/Laboratory_test_analyte_v1.yml"),
        &FirstPartyConverters,
    )
    .map_err(|diagnostics| crate::support::render(&diagnostics))?;
    let Some(Entity::Record(record)) = file.entities.first() else {
        return Err("the analyte file opens with a record".into());
    };
    let aliased: Vec<&str> = record
        .columns
        .iter()
        .filter(|column| {
            matches!(
                column.alternatives.as_slice(),
                [Alternative::Path(path)] if path.value().to_string() == "/items[at0001]"
            )
        })
        .map(Column::key)
        .collect();
    assert_eq!(
        aliased,
        [
            "operator_concept_id",
            "range_high",
            "range_low",
            "unit",
            "value"
        ]
    );
    Ok(())
}
