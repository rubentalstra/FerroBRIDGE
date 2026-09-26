// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The contract each operation gets, parameter by parameter.

use fhir_codegen::fhir::ParameterUse;

use crate::operations::contracts;
use crate::operations::find;
use crate::{R4, R4B, R5, R6};

#[test]
fn every_terminology_operation_gets_a_contract_in_both_versions() {
    for (package, module) in [(&*R4, "r4"), (&*R4B, "r4b"), (&*R5, "r5"), (&*R6, "r6")] {
        let contracts = contracts(package, module);
        let modules: Vec<&str> = contracts.iter().map(|c| c.module.as_str()).collect();
        // The R6 ballot5 core package no longer publishes CodeSystem/$find-matches
        // and ConceptMap/$closure; every earlier version does.
        let expected: Vec<&str> = if module == "r6" {
            vec![
                "code_system_lookup",
                "code_system_subsumes",
                "code_system_validate_code",
                "concept_map_translate",
                "value_set_expand",
                "value_set_validate_code",
            ]
        } else {
            vec![
                "code_system_find_matches",
                "code_system_lookup",
                "code_system_subsumes",
                "code_system_validate_code",
                "concept_map_closure",
                "concept_map_translate",
                "value_set_expand",
                "value_set_validate_code",
            ]
        };
        assert_eq!(modules, expected, "{module}");
        let lookup = find(&contracts, "CodeSystem", "lookup");
        assert_eq!(lookup.request, "CodeSystemLookupRequest");
        assert_eq!(lookup.response, "CodeSystemLookupResponse");
        assert_eq!(lookup.descriptor, "CODE_SYSTEM_LOOKUP");
        assert!(lookup.type_level && !lookup.system);
    }
}

#[test]
fn the_expand_request_is_exactly_what_each_version_declares() {
    // R4B $expand in parameters (https://hl7.org/fhir/R4B/valueset-operation-expand.html).
    let r4b = contracts(&R4B, "r4b");
    let expand = find(&r4b, "ValueSet", "expand");
    let inputs: Vec<&str> = expand.inputs.iter().map(|f| f.fhir_name.as_str()).collect();
    assert_eq!(
        inputs,
        vec![
            "url",
            "valueSet",
            "valueSetVersion",
            "context",
            "contextDirection",
            "filter",
            "date",
            "offset",
            "count",
            "includeDesignations",
            "designation",
            "includeDefinition",
            "activeOnly",
            "excludeNested",
            "excludeNotForUI",
            "excludePostCoordinated",
            "displayLanguage",
            "exclude-system",
            "system-version",
            "check-system-version",
            "force-system-version",
        ]
    );
    assert!(
        inputs
            .iter()
            .all(|name| *name != "useSupplement" && *name != "property")
    );
    assert_eq!(
        expand
            .outputs
            .iter()
            .map(|f| f.fhir_name.as_str())
            .collect::<Vec<_>>(),
        vec!["return"]
    );
    let exclude = expand
        .inputs
        .iter()
        .find(|f| f.fhir_name == "exclude-system")
        .expect("exclude-system");
    assert_eq!(exclude.name, "exclude_system");
    assert_eq!(exclude.rust_type, "super::super::primitives::Canonical");

    // R5 adds useSupplement and property (https://hl7.org/fhir/R5/valueset-operation-expand.html).
    let r5 = contracts(&R5, "r5");
    let expand = find(&r5, "ValueSet", "expand");
    let inputs: Vec<&str> = expand.inputs.iter().map(|f| f.fhir_name.as_str()).collect();
    assert!(inputs.contains(&"useSupplement"));
    assert!(inputs.contains(&"property"));
    let url = expand
        .inputs
        .iter()
        .find(|f| f.fhir_name == "url")
        .expect("url");
    assert_eq!(url.scope, vec!["type".to_owned()]);
}

#[test]
fn a_primitive_parameter_reads_the_primitives_that_specialize_it() {
    for (package, module) in [(&*R4B, "r4b"), (&*R5, "r5")] {
        let contracts = contracts(package, module);
        let expand = find(&contracts, "ValueSet", "expand");
        let accepts = |name: &str| -> Vec<String> {
            let mut list = expand
                .inputs
                .iter()
                .find(|f| f.fhir_name == name)
                .expect(name)
                .accepts
                .clone();
            list.sort();
            list
        };
        // `code`, `id`, and `markdown` specialize `string`; `canonical`, `oid`,
        // `url`, and `uuid` specialize `uri`
        // (<https://hl7.org/fhir/R5/datatypes.html#primitive>).
        assert_eq!(accepts("filter"), ["Code", "Id", "Markdown"], "{module}");
        assert_eq!(
            accepts("url"),
            ["Canonical", "Oid", "Url", "Uuid"],
            "{module}"
        );
        // An integer parameter takes nothing: `positiveInt` has another scalar.
        assert!(accepts("count").is_empty(), "{module}");
        assert!(
            accepts("includeDesignations").is_empty(),
            "{module}: boolean"
        );
    }
}

#[test]
fn multi_part_parameters_nest_and_element_maps_to_the_open_type() {
    let r4b = contracts(&R4B, "r4b");
    let lookup = find(&r4b, "CodeSystem", "lookup");
    let property = lookup
        .outputs
        .iter()
        .find(|f| f.fhir_name == "property" && f.usage == ParameterUse::Out)
        .expect("property out");
    assert_eq!(
        property.part_struct.as_deref(),
        Some("CodeSystemLookupResponseProperty")
    );
    assert_eq!(property.rust_type, "CodeSystemLookupResponseProperty");
    let names: Vec<&str> = property
        .parts
        .iter()
        .map(|p| p.fhir_name.as_str())
        .collect();
    assert_eq!(names, vec!["code", "value", "description", "subproperty"]);
    let value = property
        .parts
        .iter()
        .find(|p| p.fhir_name == "value")
        .expect("value");
    assert_eq!(value.type_code.as_deref(), Some("Element"));
    assert_eq!(
        value.rust_type,
        "super::super::parameters::ParametersParameterValue"
    );
    let subproperty = property
        .parts
        .iter()
        .find(|p| p.fhir_name == "subproperty")
        .expect("subproperty");
    assert_eq!(
        subproperty.part_struct.as_deref(),
        Some("CodeSystemLookupResponsePropertySubproperty")
    );
    assert_eq!(subproperty.parts.len(), 3);

    let r5 = contracts(&R5, "r5");
    let lookup = find(&r5, "CodeSystem", "lookup");
    let property = lookup
        .outputs
        .iter()
        .find(|f| f.fhir_name == "property" && f.usage == ParameterUse::Out)
        .expect("property out");
    let names: Vec<&str> = property
        .parts
        .iter()
        .map(|p| p.fhir_name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["code", "value", "description", "source", "subproperty"]
    );
}

#[test]
fn resource_typed_parameters_use_the_root_set_types() {
    let r4b = contracts(&R4B, "r4b");
    let validate = find(&r4b, "ValueSet", "validate-code");
    let value_set = validate
        .inputs
        .iter()
        .find(|f| f.fhir_name == "valueSet")
        .expect("valueSet");
    assert_eq!(value_set.rust_type, "super::super::value_set::ValueSet");
    assert_eq!(validate.request, "ValueSetValidateCodeRequest");
    let result = validate
        .outputs
        .iter()
        .find(|f| f.fhir_name == "result")
        .expect("result");
    assert_eq!(result.min, 1);
    let r5 = contracts(&R5, "r5");
    let validate = find(&r5, "ValueSet", "validate-code");
    let issues = validate
        .outputs
        .iter()
        .find(|f| f.fhir_name == "issues")
        .expect("R5 issues");
    assert_eq!(
        issues.rust_type,
        "super::super::operation_outcome::OperationOutcome"
    );
}
