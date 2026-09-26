// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The terminology ecosystem overlay over the declared contracts.

use fhir_codegen::closure::TypeClosure;
use fhir_codegen::lower::VersionModule;
use fhir_codegen::operations::OperationContract;
use fhir_codegen::package::Package;
use fhir_codegen::roots::RootSet;

use crate::operations::find;
use crate::{R4, R4B, R5, R6};

/// The contracts of `package` with the terminology ecosystem overlay applied,
/// the R6 package as the source of the pre-adopted parameters.
fn overlaid(package: &Package, module: &str) -> Vec<OperationContract> {
    let roots = RootSet::select(package).expect("root set selects");
    let r6_roots = RootSet::select(&R6).expect("the R6 root set selects");
    let closure = TypeClosure::compute(package, &roots).expect("closure computes");
    let model = VersionModule::lower(&closure, module, "pkg", "0").expect("model lowers");
    let mut contracts = Vec::new();
    for (url, operation) in &roots.operations {
        let source = r6_roots.operations.get(url).copied();
        for resource in &operation.resource {
            contracts.push(
                OperationContract::lower_overlaid(operation, resource, &model, source)
                    .expect("contract lowers"),
            );
        }
    }
    contracts
}

#[test]
fn the_overlay_pre_adopts_the_r6_parameters_every_earlier_version_lacks() {
    use fhir_codegen::ecosystem::ParameterSource;
    for (package, module) in [(&*R4, "r4"), (&*R4B, "r4b"), (&*R5, "r5")] {
        let contracts = overlaid(package, module);
        let validate = find(&contracts, "ValueSet", "validate-code");
        let pre_adopted: Vec<&str> = validate
            .inputs
            .iter()
            .filter(|f| f.source == ParameterSource::PreAdopted)
            .map(|f| f.fhir_name.as_str())
            .collect();
        // R5 declares `useSupplement` itself; the R4 family pre-adopts it.
        let mut expected_inputs = vec![
            "lenient-display-validation",
            "valueset-membership-only",
            "inferSystem",
            "system-version",
            "check-system-version",
            "force-system-version",
            "default-valueset-version",
            "check-valueset-version",
            "force-valueset-version",
        ];
        if module != "r5" {
            expected_inputs.insert(0, "useSupplement");
        }
        assert_eq!(pre_adopted, expected_inputs, "{module}: the R6 order");
        let outputs: Vec<(&str, ParameterSource)> = validate
            .outputs
            .iter()
            .map(|f| (f.fhir_name.as_str(), f.source))
            .collect();
        let expected_source = if module == "r5" {
            ParameterSource::Version
        } else {
            ParameterSource::PreAdopted
        };
        for name in ["code", "system", "version", "issues"] {
            assert!(
                outputs.contains(&(name, expected_source)),
                "{module}: {name} {outputs:?}"
            );
        }
        assert!(outputs.contains(&("x-caused-by-unknown-system", ParameterSource::Ecosystem)));
        let documented = validate
            .inputs
            .iter()
            .find(|f| f.fhir_name == "inferSystem")
            .expect("inferSystem");
        assert!(
            documented
                .documentation
                .as_deref()
                .unwrap()
                .starts_with("Pre-adopted from the FHIR R6 ballot"),
            "{module}: {:?}",
            documented.documentation
        );
        // $expand pre-adopts the value set version trio, and `property` where the
        // version lacks it (the ecosystem requires it on every version); $subsumes
        // gets nothing.
        let expand = find(&contracts, "ValueSet", "expand");
        let adopted: Vec<&str> = expand
            .inputs
            .iter()
            .chain(&expand.outputs)
            .filter(|f| f.source != ParameterSource::Version)
            .map(|f| f.fhir_name.as_str())
            .collect();
        let mut expected_expand = vec![
            "default-valueset-version",
            "check-valueset-version",
            "force-valueset-version",
        ];
        if module != "r5" {
            expected_expand.insert(0, "useSupplement");
            expected_expand.insert(1, "property");
        }
        assert_eq!(adopted, expected_expand, "{module}");
        let subsumes = find(&contracts, "CodeSystem", "subsumes");
        assert!(
            subsumes
                .inputs
                .iter()
                .chain(&subsumes.outputs)
                .all(|f| f.source == ParameterSource::Version),
            "{module}: CodeSystem/$subsumes"
        );
    }
}

#[test]
fn the_overlay_pre_adopts_the_r6_translate_inputs() {
    use fhir_codegen::ecosystem::ParameterSource;
    for (package, module) in [(&*R4, "r4"), (&*R4B, "r4b"), (&*R5, "r5")] {
        let contracts = overlaid(package, module);
        let translate = find(&contracts, "ConceptMap", "translate");
        let adopted: Vec<&str> = translate
            .inputs
            .iter()
            .filter(|f| f.source == ParameterSource::PreAdopted)
            .map(|f| f.fhir_name.as_str())
            .collect();
        // R5 declares the source*/target* names itself; the R4 family pre-adopts
        // them all from R6, in the R6 order.
        // R5 also takes the R6 types for `targetCode`, `targetCoding`, and
        // `targetCodeableConcept`, which it declares as `uri`.
        let expected_translate = if module == "r5" {
            vec![
                "targetCode",
                "targetCoding",
                "targetCodeableConcept",
                "sourceSystem",
                "sourceVersion",
            ]
        } else {
            vec![
                "sourceCode",
                "sourceSystem",
                "sourceVersion",
                "sourceScope",
                "sourceCoding",
                "sourceCodeableConcept",
                "targetCode",
                "targetCoding",
                "targetCodeableConcept",
                "targetScope",
                "targetSystem",
            ]
        };
        assert_eq!(adopted, expected_translate, "{module}");
    }
}

#[test]
fn the_translate_match_carries_the_ecosystem_parts_and_pre_adopts_origin_map() {
    use fhir_codegen::ecosystem::ParameterSource;
    for (package, module) in [(&*R4, "r4"), (&*R4B, "r4b"), (&*R5, "r5"), (&*R6, "r6")] {
        let contracts = overlaid(package, module);
        let translate = find(&contracts, "ConceptMap", "translate");
        let matched = translate
            .outputs
            .iter()
            .find(|f| f.fhir_name == "match")
            .expect("match");
        let part = |name: &str| {
            matched
                .parts
                .iter()
                .find(|p| p.fhir_name == name)
                .map(|p| (p.type_code.as_deref(), p.source))
        };
        // R5 declares `originMap` as a uri and takes R6's canonical; the R4
        // family pre-adopts the R6 part, and answers its own `source` as a
        // canonical, the ecosystem's type.
        let origin_source = if module == "r6" {
            ParameterSource::Version
        } else {
            ParameterSource::PreAdopted
        };
        assert_eq!(
            part("originMap"),
            Some((Some("canonical"), origin_source)),
            "{module}"
        );
        if module == "r4" || module == "r4b" {
            assert_eq!(
                part("source"),
                Some((Some("canonical"), ParameterSource::Ecosystem)),
                "{module}"
            );
        }
        for (name, type_code) in [
            ("sourceConcept", "Coding"),
            ("sourceComment", "string"),
            ("targetComment", "string"),
            ("noMap", "boolean"),
        ] {
            assert_eq!(
                part(name),
                Some((Some(type_code), ParameterSource::Ecosystem)),
                "{module}: {name}"
            );
        }
        for name in ["used-conceptmap", "used-system"] {
            let field = translate
                .outputs
                .iter()
                .find(|f| f.fhir_name == name)
                .expect(name);
            assert_eq!(
                (field.type_code.as_deref(), field.source),
                (Some("uri"), ParameterSource::Ecosystem),
                "{module}: {name}"
            );
        }
    }
}

#[test]
fn the_ecosystem_defined_outputs_join_every_version_and_r6_pre_adopts_nothing() {
    use fhir_codegen::ecosystem::ParameterSource;
    for (package, module) in [(&*R4, "r4"), (&*R4B, "r4b"), (&*R5, "r5"), (&*R6, "r6")] {
        let contracts = overlaid(package, module);
        let lookup = find(&contracts, "CodeSystem", "lookup");
        let ecosystem: Vec<(&str, Option<&str>)> = lookup
            .outputs
            .iter()
            .filter(|f| f.source == ParameterSource::Ecosystem)
            .map(|f| (f.fhir_name.as_str(), f.type_code.as_deref()))
            .collect();
        assert_eq!(
            ecosystem,
            [
                ("code", Some("code")),
                ("system", Some("uri")),
                ("abstract", Some("boolean"))
            ],
            "{module}"
        );
        for (resource, code) in [
            ("CodeSystem", "validate-code"),
            ("ValueSet", "validate-code"),
        ] {
            let contract = find(&contracts, resource, code);
            let unknown = contract
                .outputs
                .iter()
                .find(|f| f.fhir_name == "x-caused-by-unknown-system")
                .expect("x-caused-by-unknown-system");
            assert_eq!(unknown.source, ParameterSource::Ecosystem);
            assert_eq!(unknown.type_code.as_deref(), Some("canonical"));
            assert!(
                unknown
                    .documentation
                    .as_deref()
                    .unwrap()
                    .starts_with("Defined by the terminology ecosystem"),
                "{module}"
            );
        }
        if module == "r6" {
            let pre_adopted = contracts
                .iter()
                .flat_map(|c| c.inputs.iter().chain(&c.outputs))
                .filter(|f| f.source == ParameterSource::PreAdopted)
                .count();
            assert_eq!(pre_adopted, 0, "R6 is the source, it pre-adopts nothing");
        }
    }
}
