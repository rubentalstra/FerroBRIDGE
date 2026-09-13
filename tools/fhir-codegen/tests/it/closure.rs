// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

use std::collections::BTreeSet;
use std::fs;

use fhir_codegen::closure::TypeClosure;
use fhir_codegen::fhir::StructureKind;
use fhir_codegen::lower::{Cardinality, Target, TypeKind, VersionModule};
use fhir_codegen::naming::{backbone_name, type_name};
use fhir_codegen::package::Package;
use fhir_codegen::roots::{RootScope, RootSet};

use crate::{R4, R4B, R5, crate_dir, packages};

/// The wide closure and model of `package`, the emitter's own inputs.
fn wide(module: &str, package: &Package) -> (TypeClosure, VersionModule) {
    let roots =
        RootSet::select_scoped(package, RootScope::Resources).expect("the wide root set selects");
    let closure = TypeClosure::compute(package, &roots).expect("the wide closure computes");
    let model = VersionModule::lower(&closure, module, "package", "version")
        .expect("the wide model lowers");
    (closure, model)
}

/// The type modules of one version directory of the generated crate.
fn emitted_modules(module: &str) -> BTreeSet<String> {
    let dir = crate_dir().join("src").join(module);
    fs::read_dir(&dir)
        .expect("the version directory lists")
        .filter_map(|entry| {
            let path = entry.expect("the entry reads").path();
            let stem = path.file_stem()?.to_str()?.to_owned();
            (path.extension().is_some_and(|e| e == "rs") && stem != "mod" && stem != "schema")
                .then_some(stem)
        })
        .collect()
}

#[test]
fn the_emitted_tree_is_exactly_the_wide_closure() {
    for (module, _, package) in packages() {
        let (closure, model) = wide(module, package);
        for name in closure.structures().keys() {
            assert!(
                model.types.contains_key(&type_name(name)),
                "{module}: {name} is in the closure and is lowered"
            );
        }
        let modules: BTreeSet<String> = model.modules().keys().map(|m| (*m).to_owned()).collect();
        assert_eq!(
            modules,
            emitted_modules(module),
            "{module}: the emitted files are the closure's modules"
        );
    }
}

#[test]
fn a_type_is_behind_terminology_exactly_when_the_terminology_closure_holds_it() {
    for (module, _, package) in packages() {
        let roots = RootSet::select(package).expect("the terminology root set selects");
        let terminology: BTreeSet<String> = TypeClosure::compute(package, &roots)
            .expect("the terminology closure computes")
            .structures()
            .keys()
            .cloned()
            .collect();
        let (closure, model) = wide(module, package);
        for name in closure.structures().keys() {
            let expected = if terminology.contains(name) {
                RootScope::Terminology
            } else {
                RootScope::Resources
            };
            let ty = model
                .types
                .get(&type_name(name))
                .expect("every closure structure is lowered");
            assert_eq!(ty.scope, expected, "{module}: {name}");
        }
        let text = fs::read_to_string(crate_dir().join("src").join(module).join("mod.rs"))
            .expect("the version mod.rs reads");
        for name in model.modules().keys() {
            let line = format!(
                "#[cfg(feature = \"{}\")]\npub mod {name};",
                model.module_scope(name).feature()
            );
            assert!(text.contains(&line), "{module}: {name} carries its feature");
        }
    }
}

#[test]
fn the_r4_wide_closure_holds_the_clinical_resources() {
    let (closure, model) = wide("r4", &R4);
    for name in [
        "Patient",
        "Observation",
        "Condition",
        "Provenance",
        "MedicationRequest",
        "DiagnosticReport",
    ] {
        assert!(
            closure.structures().contains_key(name),
            "{name} is in the R4 wide closure"
        );
        assert_eq!(
            closure.scope(name),
            Some(RootScope::Resources),
            "{name} is behind the resources feature"
        );
        assert!(model.types.contains_key(name), "{name} is lowered");
    }
}

#[test]
fn the_resource_enum_covers_every_concrete_resource() {
    for (module, _, package) in packages() {
        let roots =
            RootSet::select_scoped(package, RootScope::Resources).expect("the root set selects");
        let (_, model) = wide(module, package);
        let TypeKind::ResourceEnum { resources } = &model
            .types
            .get("Resource")
            .expect("the Resource enum is lowered")
            .kind
        else {
            panic!("Resource is the enum");
        };
        assert_eq!(
            resources.len(),
            roots.resources.len(),
            "{module}: one variant per concrete resource"
        );
        let text = fs::read_to_string(crate_dir().join("src").join(module).join("resource.rs"))
            .expect("the resource module reads");
        assert!(
            text.contains("    Unknown(UnknownResource),"),
            "{module}: a resource outside the enabled root set is still carried"
        );
        for resource in resources {
            let feature = model
                .types
                .get(resource)
                .expect("every variant is lowered")
                .scope
                .feature();
            assert!(
                text.contains(&format!("#[cfg(feature = \"{feature}\")]\n    {resource}(")),
                "{module}: the {resource} variant carries its feature"
            );
        }
    }
}

fn closure() -> TypeClosure {
    let roots = RootSet::select(&R4B).expect("root set selects");
    TypeClosure::compute(&R4B, &roots).expect("closure computes")
}

fn model() -> VersionModule {
    VersionModule::lower(&closure(), "r4b", "hl7.fhir.r4b.core", "4.3.0").expect("model lowers")
}

#[test]
fn the_closure_holds_every_primitive_and_the_open_datatypes() {
    let closure = closure();
    let primitives: Vec<&str> = closure
        .of_kind(StructureKind::PrimitiveType)
        .map(|s| s.name.as_str())
        .collect();
    // The R4B primitive types (https://hl7.org/fhir/R4B/datatypes.html#primitive), plus xhtml.
    assert_eq!(
        primitives,
        vec![
            "base64Binary",
            "boolean",
            "canonical",
            "code",
            "date",
            "dateTime",
            "decimal",
            "id",
            "instant",
            "integer",
            "markdown",
            "oid",
            "positiveInt",
            "string",
            "time",
            "unsignedInt",
            "uri",
            "url",
            "uuid",
            "xhtml"
        ]
    );
    for name in [
        "Coding",
        "CodeableConcept",
        "Extension",
        "Meta",
        "Narrative",
        "Reference",
        "Identifier",
        "Dosage",
        "Timing",
        "Duration",
    ] {
        assert!(
            closure.structures().contains_key(name),
            "{name} is in the closure"
        );
    }
    assert_eq!(closure.roots().len(), 8);
}

#[test]
fn the_closure_stops_at_the_root_set() {
    let closure = closure();
    for name in [
        "Patient",
        "Observation",
        "ElementDefinition",
        "Resource",
        "DomainResource",
        "Element",
        "BackboneElement",
    ] {
        assert!(
            !closure.structures().contains_key(name),
            "{name} is outside the closure"
        );
    }
    assert_eq!(closure.structures().len(), 63);
}

#[test]
fn cardinality_maps_to_option_vec_and_direct() {
    let model = model();
    let TypeKind::Struct { fields } = &model.types.get("Coding").expect("Coding is lowered").kind
    else {
        panic!("Coding is a struct");
    };
    let code = fields
        .iter()
        .find(|f| f.name == "code")
        .expect("Coding.code");
    assert_eq!(code.ty.card, Cardinality::Optional);
    assert_eq!(code.ty.target, Target::Named("Code".to_owned()));
    let TypeKind::Struct { fields } = &model
        .types
        .get("ValueSetComposeInclude")
        .expect("include is lowered")
        .kind
    else {
        panic!("include is a struct");
    };
    let concept = fields
        .iter()
        .find(|f| f.name == "concept")
        .expect("include.concept");
    assert_eq!(concept.ty.card, Cardinality::Many);
    let TypeKind::Struct { fields } = &model
        .types
        .get("ValueSetComposeIncludeFilter")
        .expect("filter is lowered")
        .kind
    else {
        panic!("filter is a struct");
    };
    let op = fields.iter().find(|f| f.name == "op").expect("filter.op");
    assert_eq!(op.ty.card, Cardinality::One);
}

#[test]
fn choice_elements_become_enums() {
    let model = model();
    let value = model
        .types
        .get("ParametersParameterValue")
        .expect("choice enum exists");
    let TypeKind::Choice {
        variants,
        element_path,
    } = &value.kind
    else {
        panic!("expected a choice");
    };
    assert_eq!(element_path, "Parameters.parameter.value[x]");
    assert_eq!(variants.len(), 50);
    assert!(
        variants
            .iter()
            .any(|v| v.name == "Coding" && v.code == "Coding")
    );
    let TypeKind::Struct { fields } = &model
        .types
        .get("ParametersParameter")
        .expect("parameter")
        .kind
    else {
        panic!("parameter is a struct");
    };
    let field = fields
        .iter()
        .find(|f| f.name == "value")
        .expect("value field");
    assert_eq!(field.fhir_name, "value[x]");
    assert_eq!(
        field.ty.target,
        Target::Named("ParametersParameterValue".to_owned())
    );
}

#[test]
fn keywords_become_raw_identifiers_and_content_references_reuse_structs() {
    let model = model();
    let TypeKind::Struct { fields } = &model.types.get("ValueSetCompose").expect("compose").kind
    else {
        panic!("compose is a struct");
    };
    let exclude = fields
        .iter()
        .find(|f| f.name == "exclude")
        .expect("exclude");
    assert_eq!(
        exclude.ty.target,
        Target::Named("ValueSetComposeInclude".to_owned())
    );
    let TypeKind::Struct { fields } = &model.types.get("Identifier").expect("Identifier").kind
    else {
        panic!("Identifier is a struct");
    };
    assert!(fields.iter().any(|f| f.name == "r#use"));
    assert!(fields.iter().any(|f| f.name == "r#type"));
}

#[test]
fn the_identifier_reference_cycle_is_boxed() {
    let model = model();
    for (owner, field_name) in [("Identifier", "assigner"), ("Reference", "identifier")] {
        let TypeKind::Struct { fields } = &model.types.get(owner).expect("type exists").kind else {
            panic!("{owner} is a struct");
        };
        let field = fields
            .iter()
            .find(|f| f.name == field_name)
            .expect("field exists");
        assert!(field.ty.boxed, "{owner}.{field_name} is boxed");
    }
    let TypeKind::Struct { fields } = &model.types.get("Coding").expect("Coding").kind else {
        panic!("Coding is a struct");
    };
    assert!(fields.iter().all(|f| !f.ty.boxed));
}

#[test]
fn primitives_keep_element_id_and_extension_and_a_scalar_value() {
    let model = model();
    let boolean = model.types.get("Boolean").expect("Boolean");
    assert!(boolean.is_primitive);
    assert_eq!(boolean.module, "primitives");
    let TypeKind::Struct { fields } = &boolean.kind else {
        panic!("Boolean is a struct");
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["id", "extension", "value"]);
    assert_eq!(
        fields.get(2).map(|f| &f.ty.target),
        Some(&Target::Inline(fhir_codegen::lower::Scalar::Bool))
    );
}

#[test]
fn the_resource_enum_covers_the_root_set() {
    let model = model();
    let TypeKind::ResourceEnum { resources } =
        &model.types.get("Resource").expect("Resource enum").kind
    else {
        panic!("Resource is the enum");
    };
    assert_eq!(
        resources,
        &vec![
            "Bundle",
            "CapabilityStatement",
            "CodeSystem",
            "ConceptMap",
            "OperationOutcome",
            "Parameters",
            "TerminologyCapabilities",
            "ValueSet"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    );
    let TypeKind::Struct { fields } = &model.types.get("BundleEntry").expect("BundleEntry").kind
    else {
        panic!("BundleEntry is a struct");
    };
    let resource = fields
        .iter()
        .find(|f| f.name == "resource")
        .expect("entry.resource");
    assert_eq!(resource.ty.target, Target::Named("Resource".to_owned()));
}

fn r5_model() -> VersionModule {
    let roots = RootSet::select(&R5).expect("R5 root set selects");
    let closure = TypeClosure::compute(&R5, &roots).expect("R5 closure computes");
    VersionModule::lower(&closure, "r5", "hl7.fhir.r5.core", "5.0.0").expect("R5 model lowers")
}

#[test]
fn r5_closes_over_its_own_datatypes() {
    let roots = RootSet::select(&R5).expect("R5 root set selects");
    let closure = TypeClosure::compute(&R5, &roots).expect("R5 closure computes");
    assert_eq!(closure.roots().len(), 8);
    // R5 adds integer64 and new datatypes to the open type set
    // (https://hl7.org/fhir/R5/datatypes.html#open).
    for name in ["integer64", "Availability", "ExtendedContactDetail"] {
        assert!(
            closure.structures().contains_key(name),
            "{name} is in the R5 closure"
        );
    }
    for name in ["Contributor", "MonetaryComponent", "VirtualServiceDetail"] {
        assert!(
            !closure.structures().contains_key(name),
            "{name} is outside the R5 open types and the root set"
        );
    }
    assert_eq!(closure.structures().len(), 65);
}

#[test]
fn r5_integer64_is_an_i64_and_expansion_carries_properties() {
    let model = r5_model();
    let TypeKind::Struct { fields } = &model.types.get("Integer64").expect("Integer64").kind else {
        panic!("Integer64 is a struct");
    };
    assert_eq!(
        fields
            .iter()
            .find(|f| f.name == "value")
            .map(|f| &f.ty.target),
        Some(&Target::Inline(fhir_codegen::lower::Scalar::I64))
    );
    // ValueSet.expansion.property and contains.property are new in R5
    // (https://hl7.org/fhir/R5/valueset.html).
    assert!(model.types.contains_key("ValueSetExpansionProperty"));
    assert!(
        model
            .types
            .contains_key("ValueSetExpansionContainsProperty")
    );
    let TypeKind::Struct { fields } = &model
        .types
        .get("ValueSetExpansionContainsProperty")
        .expect("property")
        .kind
    else {
        panic!("property is a struct");
    };
    let value = fields.iter().find(|f| f.name == "value").expect("value[x]");
    assert_eq!(value.ty.card, Cardinality::One);
}

#[test]
fn every_content_reference_targets_a_type_in_a_scope_at_least_as_wide() {
    for (module, _, package) in packages() {
        // The scopes are recomputed from the narrow root set here rather than
        // read off the model, so the assertion does not rest on the value the
        // emitter itself wrote.
        let roots = RootSet::select(package).expect("the terminology root set selects");
        let narrow =
            TypeClosure::compute(package, &roots).expect("the terminology closure computes");
        let terminology: BTreeSet<String> =
            VersionModule::lower(&narrow, module, "package", "version")
                .expect("the terminology model lowers")
                .types
                .into_keys()
                .collect();
        let (_, model) = wide(module, package);
        let mut references = 0_usize;
        for ty in model.types.values() {
            let TypeKind::Struct { fields } = &ty.kind else {
                continue;
            };
            for field in fields {
                let Some(path) = field.content_reference.as_deref() else {
                    continue;
                };
                references += 1;
                let target = backbone_name(path);
                assert!(
                    model.types.contains_key(&target),
                    "{module}: {} references {path}, which lowers to {target}, a type the model does not hold",
                    field.path
                );
                if terminology.contains(&ty.name) {
                    assert!(
                        terminology.contains(&target),
                        "{module}: {} is behind terminology and references {target}, which only resources reaches",
                        field.path
                    );
                }
            }
        }
        assert!(
            references > 0,
            "{module}: the package carries content references"
        );
    }
}
