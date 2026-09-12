// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use fhir_codegen::fhir::ParameterUse;
use fhir_codegen::roots::{OPERATION_RESOURCES, ROOT_RESOURCES, RootScope, RootSet};

use crate::{R4B, packages};

/// The concrete resource types `dir` defines, read straight from the package
/// JSON: a `StructureDefinition` of kind `resource` that is neither abstract
/// nor a profile (<https://hl7.org/fhir/R4B/structuredefinition.html>).
fn concrete_resources(dir: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(dir.join("package")).expect("the package directory lists") {
        let path = entry.expect("the entry reads").path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        let text = fs::read_to_string(&path).expect("the file reads");
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let field = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
        if field("resourceType") != Some("StructureDefinition")
            || field("kind") != Some("resource")
            || field("derivation") != Some("specialization")
            || value.get("abstract").and_then(serde_json::Value::as_bool) != Some(false)
        {
            continue;
        }
        names.insert(
            field("name")
                .expect("a StructureDefinition names its type")
                .to_owned(),
        );
    }
    names
}

#[test]
fn the_wide_root_set_is_every_concrete_resource_the_package_defines() {
    for (module, dir, package) in packages() {
        let roots =
            RootSet::select_scoped(package, RootScope::Resources).expect("root set selects");
        let selected: BTreeSet<String> = roots.resources.keys().map(|n| (*n).to_owned()).collect();
        assert_eq!(selected, concrete_resources(&dir), "{module}");
        for name in ROOT_RESOURCES {
            assert_eq!(
                roots.scope(name),
                Some(RootScope::Terminology),
                "{module}: {name} comes from the terminology root set"
            );
        }
        // The R4 resource count the widening was declared with (#120).
        if module == "r4" {
            assert_eq!(selected.len(), 146, "the R4 concrete resource count");
        }
    }
}

#[test]
fn the_terminology_root_set_is_a_subset_of_the_wide_one() {
    for (module, _, package) in packages() {
        let narrow = RootSet::select(package).expect("terminology root set selects");
        let wide =
            RootSet::select_scoped(package, RootScope::Resources).expect("wide root set selects");
        assert_eq!(
            narrow.resources.keys().copied().collect::<Vec<_>>(),
            ROOT_RESOURCES.to_vec(),
            "{module}"
        );
        assert!(!narrow.holds(RootScope::Resources), "{module}");
        assert!(wide.holds(RootScope::Resources), "{module}");
        for name in narrow.resources.keys() {
            assert!(wide.resources.contains_key(name), "{module}: {name}");
        }
        assert_eq!(
            narrow.operations.keys().collect::<Vec<_>>(),
            wide.operations.keys().collect::<Vec<_>>(),
            "{module}: widening the resources leaves the operations alone"
        );
    }
}

#[test]
fn the_eight_root_resources_are_found() {
    let roots = RootSet::select(&R4B).expect("root set selects");
    assert_eq!(
        roots.resources.keys().copied().collect::<Vec<_>>(),
        ROOT_RESOURCES.to_vec()
    );
    for (name, definition) in &roots.resources {
        assert_eq!(&definition.name, name);
        assert_eq!(&definition.type_name, name);
    }
}

#[test]
fn exactly_the_terminology_operations_are_selected() {
    let roots = RootSet::select(&R4B).expect("root set selects");
    let ids = roots
        .operations
        .values()
        .map(|operation| format!("{}${}", operation.resource.join(","), operation.code))
        .collect::<Vec<_>>();
    // The R4B operations defined on CodeSystem, ValueSet, and ConceptMap
    // (https://hl7.org/fhir/R4B/terminology-module.html).
    assert_eq!(
        ids,
        vec![
            "CodeSystem$find-matches",
            "CodeSystem$lookup",
            "CodeSystem$subsumes",
            "CodeSystem$validate-code",
            "ConceptMap$closure",
            "ConceptMap$translate",
            "ValueSet$expand",
            "ValueSet$validate-code",
        ]
    );
    for operation in roots.operations.values() {
        for resource in &operation.resource {
            assert!(OPERATION_RESOURCES.contains(&resource.as_str()));
        }
    }
}

#[test]
fn operations_are_looked_up_by_resource_and_code() {
    let roots = RootSet::select(&R4B).expect("root set selects");
    let lookup = roots
        .operation("CodeSystem", "lookup")
        .expect("$lookup exists");
    assert_eq!(
        lookup.url,
        "http://hl7.org/fhir/OperationDefinition/CodeSystem-lookup"
    );
    let outputs = lookup
        .parameter
        .iter()
        .filter(|parameter| parameter.usage == ParameterUse::Out)
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();
    // The R4B $lookup out parameters
    // (https://hl7.org/fhir/R4B/codesystem-operation-lookup.html).
    assert_eq!(
        outputs,
        vec!["name", "version", "display", "designation", "property"]
    );
    let property = lookup
        .parameter
        .iter()
        .find(|parameter| parameter.name == "property" && parameter.usage == ParameterUse::Out)
        .expect("the property out parameter exists");
    assert_eq!(
        property
            .part
            .iter()
            .map(|part| part.name.as_str())
            .collect::<Vec<_>>(),
        vec!["code", "value", "description", "subproperty"]
    );
    assert!(roots.operation("ValueSet", "lookup").is_none());
    assert!(roots.operation("Resource", "validate").is_none());
}
