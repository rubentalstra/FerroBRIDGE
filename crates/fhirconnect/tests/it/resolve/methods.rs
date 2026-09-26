// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The concept methods, the conditions, the `manual` entries, the
//! preprocessors and the names a mapping takes.

use core::error::Error;

use fhirconnect::resolve::program::hierarchy::Create;
use fhirconnect::resolve::program::manual::ManualPath;
use fhirconnect::resolve::program::manual::ManualValue;
use fhirconnect::resolve::program::mapping::Method;
use fhirconnect::resolve::program::target::Attachment;
use openehr_mapping_core::diagnostic::Diagnostic;

use crate::resolve::PROBLEM;
use crate::resolve::borrow;
use crate::resolve::codes;
use crate::resolve::context;
use crate::resolve::extension;
use crate::resolve::model;
use crate::resolve::program_of;
use crate::resolve::refusals;
use crate::resolve::render;
use crate::resolve::set_of;
use crate::resolve::start_context;
use crate::resolve::start_model;

/// A mapping method is one of the concept types
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`),
/// so a method writing two of them says nothing about which one runs.
#[test]
fn a_mapping_writing_two_mapping_methods_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    link:\n      meaning: \"follow up\"\n    participationsFunction: \"performer\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-conflicting-mapping-methods"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("link"), "{message}");
    assert!(message.contains("participationsFunction"), "{message}");
    Ok(())
}

/// "Conditions can also be unattached to the path contained in the `with:`
/// method and point to a different path ... these paths are handled as simple
/// true/false"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §targetAttribute), so the compiler decides the attachment rather than
/// leaving the engine to compare path text.
#[test]
fn an_unattached_condition_compiles_as_unrelated() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    fhirCondition:\n      targetRoot: \"$resource.verificationStatus\"\n      targetAttribute: \"coding.code\"\n      operator: \"one of\"\n      criteria: \"confirmed\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let mapping = program.mappings().first().ok_or("one mapping")?;
    let condition = mapping.fhir_condition().ok_or("the condition")?;
    assert_eq!(condition.attachment(), Attachment::Unrelated);
    Ok(())
}

/// `$context` "holds values passed in on the REST call ... a context value is
/// referenced from a `manual` `value`"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`,
/// §`$context`), so a compiled manual value says which of the two it is.
#[test]
fn a_manual_value_is_a_literal_or_a_context_member() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    manual:\n      - name: \"who\"\n        fhir:\n          - path: \"$fhirRoot.text\"\n            value: \"$context.who\"\n          - path: \"$fhirRoot.id\"\n            value: \"fixed\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let mapping = program.mappings().first().ok_or("one mapping")?;
    let entry = mapping.manual().first().ok_or("the manual entry")?;
    let values: Vec<&ManualValue> = entry.fhir().iter().map(ManualPath::value).collect();
    assert_eq!(
        values,
        vec![
            &ManualValue::Context(String::from("who")),
            &ManualValue::Literal(String::from("fixed"))
        ]
    );
    Ok(())
}

#[test]
fn a_manual_value_naming_no_context_member_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    manual:\n      - name: \"who\"\n        fhir:\n          - path: \"$fhirRoot.text\"\n            value: \"$context\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-malformed-context-value"]);
    Ok(())
}

/// "The `create` method defines the element to create. In the case of a
/// `resource` or `archetype`, this type is inferred by the FHIRconnect mapping
/// file it is included in"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/HierarchyMappings.adoc`,
/// §split), and the worked example writes `event` for the openEHR side.
#[test]
fn a_split_creates_one_of_the_three_elements() -> Result<(), Box<dyn Error>> {
    let body = "preprocessor:\n  hierarchy:\n    with:\n      fhir: \"$resource.category\"\n      openehr: \"$archetype/data[at0001]\"\n    split:\n      fhir:\n        create: \"resource\"\nmappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let hierarchy = program.hierarchy().ok_or("the hierarchy mapping")?;
    let split = hierarchy.split_fhir().ok_or("the FHIR side of the split")?;
    assert_eq!(split.create(), Some(Create::Resource));
    Ok(())
}

#[test]
fn a_split_creating_an_element_the_engine_does_not_make_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "preprocessor:\n  hierarchy:\n    with:\n      fhir: \"$resource.category\"\n      openehr: \"$archetype/data[at0001]\"\n    split:\n      fhir:\n        create: \"cluster\"\nmappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-split-create"]);
    Ok(())
}

/// A diagnostic names the place in the document it is about, so a refusal
/// about a nested method carries its own depth rather than the index of the
/// top-level method it hangs under.
#[test]
fn a_nested_refusal_names_its_own_depth() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    followedBy:\n      mappings:\n        - name: \"child\"\n          with:\n            fhir: \"$fhirRoot.text\"\n            openehr: \"$archetype/data[at0001]/items[at9999]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-template-node"]);
    let diagnostic = diagnostics.first().ok_or("one refusal")?;
    assert_eq!(
        diagnostic.model_path().to_string(),
        "mappings[0].followedBy.mappings[0].with.openehr"
    );
    Ok(())
}

#[test]
fn a_mapping_code_the_engine_does_not_register_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    mappingCode: \"absentFunction\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-context-reference"]);
    Ok(())
}

/// "This logic defines that the mapping file is only executed if the given
/// condition is met"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §Conditions in the preprocessor), so a slotted file keeps its own gate.
#[test]
fn the_preprocessor_of_a_slotted_model_reaches_the_slot() -> Result<(), Box<dyn Error>> {
    let start = "mappings:\n  - name: \"qualifier\"\n    with:\n      fhir: \"$fhirRoot\"\n      openehr: \"$archetype/data[at0001]/items[openEHR-EHR-CLUSTER.problem_qualifier.v2]\"\n    slotArchetype: \"CLUSTER.synthetic.v2\"\n";
    let slotted = "preprocessor:\n  openehrCondition:\n    targetRoot: \"$archetype\"\n    targetAttribute: \"items[at0063]\"\n    operator: \"not empty\"\nmappings:\n  - name: \"code\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/items[at0063]\"\n";
    let files = [
        ("model.yml", start_model(start)),
        (
            "slotted.yml",
            model(
                "CLUSTER.synthetic.v2",
                "openEHR-EHR-CLUSTER.problem_qualifier.v2",
                slotted,
            ),
        ),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let method = program
        .mappings()
        .first()
        .ok_or("the slot mapping")?
        .method();
    let Method::Slot {
        ref preprocessors, ..
    } = *method
    else {
        return Err(format!("the mapping compiled to {method:?}").into());
    };
    let gate = preprocessors.first().ok_or("the slotted file's gate")?;
    assert_eq!(gate.model().as_str(), "CLUSTER.synthetic.v2");
    assert!(gate.openehr_condition().is_some());
    assert!(!gate.is_empty());
    Ok(())
}

/// An extension file is a file, so its own preprocessor gates the program
/// beside the start model mapping's.
#[test]
fn the_preprocessor_of_an_extension_reaches_the_program() -> Result<(), Box<dyn Error>> {
    let body = "preprocessor:\n  fhirCondition:\n    targetRoot: \"$resource.verificationStatus\"\n    targetAttribute: \"coding\"\n    operator: \"not empty\"\nmappings:\n  - name: \"note\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n";
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension("synthetic_extension", "EVALUATION.synthetic.v1", body),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let names: Vec<&str> = program
        .preprocessors()
        .iter()
        .map(|preprocessor| preprocessor.model().as_str())
        .collect();
    assert_eq!(names, vec!["synthetic_extension"]);
    assert!(program.preprocessor().is_none(), "the start model has none");
    let gate = program.preprocessors().first().ok_or("the gate")?;
    assert!(gate.fhir_condition().is_some());
    Ok(())
}

/// `overwrite` "overwrites the mapping method with the same name"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`),
/// so two top-level methods sharing a name leave an overwrite no single
/// target, and the refusal names where both are.
#[test]
fn two_methods_of_one_file_sharing_a_name_are_refused_naming_both() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      \
                openehr: \"$archetype/data[at0001]/items[at0002]\"\n  - name: \"problem\"\n    \
                with:\n      fhir: \"$resource.note.text\"\n      openehr: \
                \"$archetype/data[at0001]/items[at0069]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-duplicate-method-name"]);
    let message = diagnostics
        .first()
        .map(Diagnostic::message)
        .ok_or("a refusal")?;
    assert!(
        message.contains("line 14 column 11") && message.contains("line 18 column 11"),
        "the refusal names both positions: {message}"
    );
    Ok(())
}

/// `appendTo` reaches a child by its dotted name, so two siblings of one
/// `followedBy` sharing a name are refused the same way.
#[test]
fn two_sibling_children_sharing_a_name_are_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource\"\n      \
                openehr: \"$archetype\"\n      type: \"NONE\"\n    followedBy:\n      mappings:\n        \
                - name: \"code\"\n          with:\n            fhir: \"code\"\n            openehr: \
                \"data[at0001]/items[at0002]\"\n        - name: \"code\"\n          with:\n            \
                fhir: \"note.text\"\n            openehr: \"data[at0001]/items[at0069]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-duplicate-method-name"]);
    Ok(())
}
