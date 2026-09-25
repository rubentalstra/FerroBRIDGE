// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Compiling mapping sets against the synthetic laboratory template: the
//! corpus laboratory files bind, and every structural fault refuses the
//! compile with a typed error naming its file and entry.

use std::error::Error;
use std::sync::Arc;

use omocl::engine::custom::Converters;
use omocl::engine::custom::CustomConverter;
use omocl::model::semantic::ConverterRegistry;
use omocl::model::semantic::FirstPartyConverters;
use omocl::resolve::ResolveError;
use omocl::resolve::compile;

use crate::lab::LABORATORY;
use crate::lab::compiled;
use crate::lab::set;
use crate::lab::template;

/// Returns an inline OMOCL file mapping `archetype` with `mappings`.
pub(crate) fn mapping(name: &str, archetype: &str, mappings: &str) -> String {
    format!(
        "grammar: OMOCL/v1.0.0\ntype: model\nmetadata:\n  name: {name}\n  version: 1.0.0\n\
         spec:\n  system: OMOP\n  version: 5.4\n  openEhrConfig:\n    archetype: {archetype}\n\
         mappings:\n{mappings}"
    )
}

/// The analyte archetype.
pub(crate) const ANALYTE: &str = "openEHR-EHR-CLUSTER.laboratory_test_analyte.v1";

/// The laboratory result archetype.
pub(crate) const RESULT: &str = "openEHR-EHR-OBSERVATION.laboratory_test_result.v1";

#[test]
fn the_laboratory_files_compile_against_the_laboratory_template() -> Result<(), Box<dyn Error>> {
    let index = template()?;
    let program = compiled(&set(LABORATORY, &[])?, &index)?;
    insta::assert_snapshot!("laboratory_program", program.to_string());
    Ok(())
}

#[test]
fn an_include_the_template_carries_no_node_for_is_recorded_unbound() -> Result<(), Box<dyn Error>> {
    let index = template()?;
    let program = compiled(&set(LABORATORY, &[])?, &index)?;
    let includes: Vec<String> = program
        .unbound()
        .iter()
        .filter(|unbound| unbound.part == "Include")
        .map(|unbound| format!("{}#{}", unbound.mapping, unbound.entry))
        .collect();
    assert_eq!(includes, vec!["Laboratory_test_result_v1#2".to_owned()]);
    Ok(())
}

#[test]
fn the_same_archetype_is_bound_once_below_its_include() -> Result<(), Box<dyn Error>> {
    let index = template()?;
    let program = compiled(&set(LABORATORY, &[])?, &index)?;
    let roots: Vec<&str> = program
        .roots()
        .iter()
        .map(|scope| scope.mapping().as_str())
        .collect();
    assert_eq!(roots, vec!["Laboratory_test_result_v1"]);
    Ok(())
}

#[test]
fn an_include_cycle_refuses_the_compile() -> Result<(), Box<dyn Error>> {
    let index = template()?;
    let back = mapping(
        "Synthetic_cycle_v1",
        ANALYTE,
        &format!(
            "  - type: \"Include\"\n    base_path: \"/items[{RESULT}]\"\n    archetype_id: \
             \"{RESULT}\"\n"
        ),
    );
    let set = set(LABORATORY, &[("Synthetic_cycle_v1.yml", &back)])?;
    let errors = compile(&set, &index, &FirstPartyConverters).expect_err("a cycle");
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, ResolveError::IncludeCycle { chain } if chain.len() == 3)),
        "{errors:?}"
    );
    Ok(())
}

/// A registry that admits a name and holds no object for it.
#[derive(Debug)]
struct Hollow;

impl ConverterRegistry for Hollow {
    fn contains(&self, _name: &str) -> bool {
        true
    }
}

impl Converters for Hollow {
    fn converter(&self, _name: &str) -> Option<Arc<dyn CustomConverter>> {
        None
    }
}

#[test]
fn a_converter_with_no_object_refuses_the_compile() -> Result<(), Box<dyn Error>> {
    let index = template()?;
    let errors = compile(&set(LABORATORY, &[])?, &index, &Hollow).expect_err("no converter");
    assert_eq!(
        errors,
        vec![ResolveError::UnknownConverter {
            mapping: "Laboratory_test_result_v1".to_owned(),
            entry: 3,
            name: "FactRelationshipCustomConverter".to_owned(),
        }]
    );
    Ok(())
}

#[test]
fn a_path_variable_refuses_the_compile() -> Result<(), Box<dyn Error>> {
    let index = template()?;
    let file = mapping(
        "Synthetic_variable_v1",
        ANALYTE,
        "  - type: \"Measurement\"\n    concept_id:\n      alternatives:\n        - path: \
         \"$archetype/items[at0024]\"\n    measurement_date:\n      alternatives:\n        - \
         path: \"/items[at0025]\"\n",
    );
    let errors = compile(
        &set(&[], &[("Synthetic_variable_v1.yml", &file)])?,
        &index,
        &FirstPartyConverters,
    )
    .expect_err("a variable");
    assert!(
        matches!(
            errors.as_slice(),
            [ResolveError::PathVariable { entry: 0, .. }]
        ),
        "{errors:?}"
    );
    Ok(())
}

#[test]
fn a_required_column_no_alternative_of_which_binds_refuses_the_compile()
-> Result<(), Box<dyn Error>> {
    let index = template()?;
    let file = mapping(
        "Synthetic_unbound_v1",
        ANALYTE,
        "  - type: \"Measurement\"\n    concept_id:\n      alternatives:\n        - path: \
         \"/items[at9999]\"\n    measurement_date:\n      alternatives:\n        - path: \
         \"/items[at0025]\"\n",
    );
    let errors = compile(
        &set(&[], &[("Synthetic_unbound_v1.yml", &file)])?,
        &index,
        &FirstPartyConverters,
    )
    .expect_err("an unbound column");
    assert!(
        matches!(
            errors.as_slice(),
            [ResolveError::UnboundRequiredColumn {
                column: "concept_id",
                ..
            }]
        ),
        "{errors:?}"
    );
    Ok(())
}

#[test]
fn an_optional_column_that_names_no_node_is_recorded_unbound() -> Result<(), Box<dyn Error>> {
    let index = template()?;
    let file = mapping(
        "Synthetic_optional_v1",
        ANALYTE,
        "  - type: \"Measurement\"\n    concept_id:\n      alternatives:\n        - path: \
         \"/items[at0024]\"\n    measurement_date:\n      alternatives:\n        - path: \
         \"/items[at0025]\"\n    unit:\n      optional: true\n      alternatives:\n        - \
         path: \"/items[at9999]\"\n",
    );
    let program = compiled(&set(&[], &[("Synthetic_optional_v1.yml", &file)])?, &index)?;
    let parts: Vec<(&str, &str)> = program
        .unbound()
        .iter()
        .map(|unbound| (unbound.part.as_str(), unbound.path.as_str()))
        .collect();
    assert_eq!(parts, vec![("unit", "/items[at9999]")]);
    Ok(())
}

#[test]
fn the_self_step_names_the_anchor() -> Result<(), Box<dyn Error>> {
    let index = template()?;
    let file = mapping(
        "Synthetic_self_v1",
        RESULT,
        "  - type: \"Measurement\"\n    base_path: \"/data[at0001]/events[at0002]\"\n    \
         concept_id:\n      alternatives:\n        - code: 1001\n    measurement_date:\n      \
         alternatives:\n        - path: \".\"\n",
    );
    let program = compiled(&set(&[], &[("Synthetic_self_v1.yml", &file)])?, &index)?;
    let rendered = program.to_string();
    assert!(
        rendered.contains("measurement_date: path up 0 down ."),
        "{rendered}"
    );
    Ok(())
}
