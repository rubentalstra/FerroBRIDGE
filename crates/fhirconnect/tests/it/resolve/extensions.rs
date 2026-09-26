// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The extensions a context applies: the collision, order and reach rules.

use core::error::Error;
use fhirconnect::model::ast::keyword::Direction;

use fhirconnect::resolve::program::mapping::Mapping;
use openehr_mapping_core::diagnostic::Diagnostic;

use crate::resolve::PROBLEM;
use crate::resolve::borrow;
use crate::resolve::codes;
use crate::resolve::context;
use crate::resolve::extension;
use crate::resolve::loaded_set;
use crate::resolve::model;
use crate::resolve::program_of;
use crate::resolve::refusals;
use crate::resolve::render;
use crate::resolve::set_of;
use crate::resolve::start_context;
use crate::resolve::start_model;

#[test]
fn an_add_whose_name_collides_is_refused_naming_both_mappings() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "EVALUATION.synthetic.v1",
                "mappings:\n  - name: \"problem\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
            ),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-add-name-collision"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("EVALUATION.synthetic.v1"), "{message}");
    assert!(message.contains("synthetic_extension"), "{message}");
    Ok(())
}

#[test]
fn an_append_that_carries_mapping_logic_is_refused() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "EVALUATION.synthetic.v1",
                "mappings:\n  - name: \"extra\"\n    extension: \"append\"\n    appendTo: \"problem\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
            ),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-append-carries-mapping"]);
    Ok(())
}

/// "If more logic needs to be altered, the overwrite method must be used
/// instead"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Append), so a concept key on an `append` is refused rather than dropped.
#[test]
fn an_append_carrying_a_concept_key_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"extra\"\n    extension: \"append\"\n    appendTo: \"problem\"\n    slotArchetype: \"CLUSTER.absent.v1\"\n    mappingCode: \"someFunction\"\n    conceptmap: \"http://example.org/fhir/ConceptMap/x\"\n    unidirectional: \"openehr->fhir\"\n    followedBy:\n      mappings:\n        - name: \"child\"\n          with:\n            fhir: \"$resource.note.text\"\n            openehr: \"$archetype/data[at0001]/items[at0069]\"\n";
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
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(
        codes(&diagnostics),
        vec![
            "fc-append-carries-mapping",
            "fc-append-carries-mapping",
            "fc-append-carries-mapping",
            "fc-append-carries-mapping"
        ]
    );
    let named: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.model_path().to_string())
        .collect();
    for key in [
        "slotArchetype",
        "mappingCode",
        "conceptmap",
        "unidirectional",
    ] {
        assert!(
            named.iter().any(|path| path.ends_with(key)),
            "{key} in {named:?}"
        );
    }
    Ok(())
}

/// An `appendTo` is resolved against the model mapping the extensions of the
/// context have been applied to, so a method one extension `add`s is a target
/// the next extension may `append` to
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Append). This case takes the whole loader, so the strict schema and the
/// semantic rules run over it as they do on a real load.
#[test]
fn an_append_to_a_method_another_extension_added_loads_and_compiles() -> Result<(), Box<dyn Error>>
{
    let added = "mappings:\n  - name: \"note\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n";
    let appended = "mappings:\n  - name: \"extra\"\n    extension: \"append\"\n    appendTo: \"note\"\n    followedBy:\n      mappings:\n        - name: \"authored\"\n          with:\n            fhir: \"$fhirRoot.id\"\n            openehr: \"$openehrRoot\"\n";
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "adds.yml",
            extension("synthetic_add", "EVALUATION.synthetic.v1", added),
        ),
        (
            "appends.yml",
            extension("synthetic_append", "EVALUATION.synthetic.v1", appended),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_add", "synthetic_append"]),
            ),
        ),
    ];
    let (_directory, set) = loaded_set(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let names: Vec<&str> = program.mappings().iter().map(Mapping::name).collect();
    assert_eq!(names, vec!["problem", "note"]);
    let note = program.mappings().get(1).ok_or("the added method")?;
    let child = note.followed_by().first().ok_or("the appended child")?;
    assert_eq!(child.name(), "note.authored");
    Ok(())
}

#[test]
fn an_append_without_a_target_is_refused() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "EVALUATION.synthetic.v1",
                "mappings:\n  - name: \"extra\"\n    extension: \"append\"\n    followedBy:\n      mappings:\n        - name: \"child\"\n          with:\n            fhir: \"$resource.note.text\"\n            openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
            ),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-append-without-target"]);
    Ok(())
}

#[test]
fn an_append_to_naming_no_method_of_the_model_is_refused() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "EVALUATION.synthetic.v1",
                "mappings:\n  - name: \"extra\"\n    extension: \"append\"\n    appendTo: \"absent\"\n    followedBy:\n      mappings:\n        - name: \"child\"\n          with:\n            fhir: \"$resource.note.text\"\n            openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
            ),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-extension-target"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("absent"), "{message}");
    Ok(())
}

/// "The value used is the name of the mapping method. This can be also the
/// child method. The path then would be `appendTo: parent.child`"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Append).
#[test]
fn an_append_to_reaches_a_nested_method_by_its_dotted_name() -> Result<(), Box<dyn Error>> {
    let parent = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    followedBy:\n      mappings:\n        - name: \"coding\"\n          with:\n            fhir: \"coding\"\n            openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
    let files = [
        ("model.yml", start_model(parent)),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "EVALUATION.synthetic.v1",
                "mappings:\n  - name: \"extra\"\n    extension: \"append\"\n    appendTo: \"problem.coding\"\n    followedBy:\n      mappings:\n        - name: \"display\"\n          with:\n            fhir: \"display\"\n            openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
            ),
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
    let appended = program
        .mappings()
        .first()
        .ok_or("the parent mapping")?
        .followed_by()
        .first()
        .ok_or("the child mapping")?
        .followed_by()
        .first()
        .ok_or("the appended mapping")?;
    assert_eq!(appended.name(), "problem.coding.display");
    Ok(())
}

#[test]
fn an_extension_method_naming_no_method_of_the_model_is_refused() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "EVALUATION.synthetic.v1",
                "mappings:\n  - name: \"absent\"\n    extension: \"overwrite\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
            ),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-extension-target"]);
    Ok(())
}

#[test]
fn two_extensions_overwriting_one_name_are_refused_naming_both() -> Result<(), Box<dyn Error>> {
    let overwrite = "mappings:\n  - name: \"problem\"\n    extension: \"overwrite\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n";
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "first.yml",
            extension("first_extension", "EVALUATION.synthetic.v1", overwrite),
        ),
        (
            "second.yml",
            extension("second_extension", "EVALUATION.synthetic.v1", overwrite),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["first_extension", "second_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-repeated-overwrite"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("first_extension"), "{message}");
    assert!(message.contains("second_extension"), "{message}");
    Ok(())
}

#[test]
fn the_declaration_order_of_two_extensions_decides_the_result() -> Result<(), Box<dyn Error>> {
    let first = extension(
        "first_extension",
        "EVALUATION.synthetic.v1",
        "mappings:\n  - name: \"note\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
    );
    let second = extension(
        "second_extension",
        "EVALUATION.synthetic.v1",
        "mappings:\n  - name: \"severity\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.severity\"\n      openehr: \"$archetype/data[at0001]/items[at0005]\"\n",
    );
    let model = start_model(PROBLEM);
    let names = |extensions: &[&str]| -> Result<Vec<String>, Box<dyn Error>> {
        let declaration = context("synthetic.context", &start_context(extensions));
        let set = set_of(&[
            ("model.yml", model.as_str()),
            ("first.yml", first.as_str()),
            ("second.yml", second.as_str()),
            ("context.yml", declaration.as_str()),
        ])?;
        let program =
            program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
        Ok(program
            .mappings()
            .iter()
            .map(|mapping| mapping.name().to_owned())
            .collect())
    };
    assert_eq!(
        names(&["first_extension", "second_extension"])?,
        vec!["problem", "note", "severity"]
    );
    assert_eq!(
        names(&["second_extension", "first_extension"])?,
        vec!["problem", "severity", "note"]
    );
    Ok(())
}

#[test]
fn a_nested_extension_method_is_refused() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "EVALUATION.synthetic.v1",
                "mappings:\n  - name: \"note\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.note\"\n      openehr: \"$archetype\"\n    followedBy:\n      mappings:\n        - name: \"text\"\n          extension: \"add\"\n          with:\n            fhir: \"text\"\n            openehr: \"data[at0001]/items[at0069]\"\n",
            ),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-nested-extension-method"]);
    Ok(())
}

#[test]
fn a_method_of_an_extension_file_without_a_method_key_is_refused() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "EVALUATION.synthetic.v1",
                "mappings:\n  - name: \"note\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
            ),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-extension-method-missing"]);
    Ok(())
}

/// "The concept map can also be directly attached inside the header, this way
/// all codes contained in the conceptmap will be transformed using the
/// conceptmap"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/manual.adoc`,
/// §`ConceptMaps`), so a file-level `spec.conceptmap` reaches every method of
/// that file and the method's own wins where it writes one.
#[test]
fn a_file_level_conceptmap_reaches_every_method_of_the_file() -> Result<(), Box<dyn Error>> {
    let model = format!(
        "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: EVALUATION.synthetic.v1\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  conceptmap: \"http://example.org/fhir/ConceptMap/header\"\n  openEhrConfig:\n    archetype: openEHR-EHR-EVALUATION.problem_diagnosis.v1\n  fhirConfig:\n    structureDefinition: http://hl7.org/fhir/StructureDefinition/Condition\n{}",
        "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    followedBy:\n      mappings:\n        - name: \"severity\"\n          conceptmap: \"http://example.org/fhir/ConceptMap/method\"\n          with:\n            fhir: \"$fhirRoot.text\"\n            openehr: \"$openehrRoot\"\n"
    );
    let files = [
        ("model.yml", model),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let problem = program.mappings().first().ok_or("one mapping")?;
    assert_eq!(
        problem.conceptmap(),
        Some("http://example.org/fhir/ConceptMap/header")
    );
    let severity = problem.followed_by().first().ok_or("the child")?;
    assert_eq!(
        severity.conceptmap(),
        Some("http://example.org/fhir/ConceptMap/method")
    );
    Ok(())
}

/// An extension file carries its own `spec`, so the `unidirectional` it writes
/// pins the methods it contributes and leaves the extended file's alone
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Direction).
#[test]
fn the_direction_an_extension_file_pins_reaches_the_methods_it_adds() -> Result<(), Box<dyn Error>>
{
    let contributed = format!(
        "grammar: FHIRConnect/v1.0.0\ntype: extension\nmetadata:\n  name: synthetic_extension\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  unidirectional: \"fhir->openehr\"\n  conceptmap: \"http://example.org/fhir/ConceptMap/extension\"\n  extends: EVALUATION.synthetic.v1\n{}",
        "mappings:\n  - name: \"note\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.category.first().coding\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n"
    );
    let files = [
        ("model.yml", start_model(PROBLEM)),
        ("extension.yml", contributed),
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
    let problem = program.mappings().first().ok_or("the model's method")?;
    assert_eq!(problem.direction(), None);
    assert_eq!(problem.conceptmap(), None);
    let note = program.mappings().get(1).ok_or("the added method")?;
    assert_eq!(note.direction(), Some(Direction::FhirToOpenehr));
    assert_eq!(
        note.conceptmap(),
        Some("http://example.org/fhir/ConceptMap/extension")
    );
    Ok(())
}

/// `context.extensions` names extension files, so a model file listed there
/// is refused rather than applied
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/contextual-mapping.adoc`).
#[test]
fn a_model_file_listed_under_extensions_is_refused() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "other.yml",
            model(
                "CLUSTER.synthetic.v2",
                "openEHR-EHR-CLUSTER.problem_qualifier.v2",
                PROBLEM,
            ),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["CLUSTER.synthetic.v2"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-not-an-extension"]);
    Ok(())
}

/// An extension file states what it extends with `spec.extends`
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-mapping.adoc`),
/// so one that names nothing has no target to apply to.
#[test]
fn an_extension_that_extends_nothing_is_refused() -> Result<(), Box<dyn Error>> {
    let orphan = String::from(
        "grammar: FHIRConnect/v1.0.0\ntype: extension\nmetadata:\n  name: synthetic_extension\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\nmappings:\n  - name: \"note\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at0069]\"\n",
    );
    let files = [
        ("model.yml", start_model(PROBLEM)),
        ("extension.yml", orphan),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-extension-without-target"]);
    Ok(())
}

/// A method an extension contributes belongs to the extension's file, so a
/// refusal about it names that file and the place it sits there, never an
/// index into the merged model mapping.
#[test]
fn a_refusal_about_an_added_method_names_the_extension_file() -> Result<(), Box<dyn Error>> {
    let added = "mappings:\n  - name: \"note\"\n    extension: \"add\"\n    with:\n      fhir: \"$resource.note.text\"\n      openehr: \"$archetype/data[at0001]/items[at9999]\"\n";
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "extension.yml",
            extension("synthetic_extension", "EVALUATION.synthetic.v1", added),
        ),
        (
            "context.yml",
            context(
                "synthetic.context",
                &start_context(&["synthetic_extension"]),
            ),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-template-node"]);
    let diagnostic = diagnostics.first().ok_or("one refusal")?;
    assert_eq!(
        diagnostic.file().to_string_lossy().as_ref(),
        "extension.yml"
    );
    assert_eq!(
        diagnostic.model_path().to_string(),
        "mappings[0].with.openehr"
    );
    Ok(())
}

/// An extension applies when the model it extends is merged, so one whose
/// model no slot reaches changes nothing; the context compiles and the
/// program carries a warning naming the extension and the model.
#[test]
fn a_listed_extension_whose_model_is_never_reached_is_a_warning() -> Result<(), Box<dyn Error>> {
    let unreached = model(
        "CLUSTER.synthetic.v1",
        "openEHR-EHR-CLUSTER.problem_qualifier.v2",
        "mappings:\n  - name: \"status\"\n    with:\n      fhir: \"$resource.clinicalStatus\"\n      \
         openehr: \"$archetype/items[at0003]\"\n",
    );
    let files = [
        ("model.yml", start_model(PROBLEM)),
        ("cluster.yml", unreached),
        (
            "extension.yml",
            extension(
                "synthetic_extension",
                "CLUSTER.synthetic.v1",
                "mappings:\n  - name: \"status\"\n    extension: \"overwrite\"\n    with:\n      \
                 fhir: \"$resource.clinicalStatus\"\n      openehr: \"$archetype/items[at0003]\"\n",
            ),
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
    let warnings = program.warnings();
    assert_eq!(codes(warnings), vec!["fc-unreached-extension"]);
    let message = warnings
        .first()
        .map(Diagnostic::message)
        .ok_or("a warning")?;
    assert!(
        message.contains("`synthetic_extension`") && message.contains("`CLUSTER.synthetic.v1`"),
        "the warning names the extension and the model: {message}"
    );
    assert_eq!(
        warnings.first().map(Diagnostic::severity),
        Some(openehr_mapping_core::diagnostic::Severity::Warning)
    );
    Ok(())
}
