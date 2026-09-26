// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The paths on both sides: the anchors, the grammar, the read-only forms,
//! the template nodes and the reference-model tails.

use core::error::Error;
use fhirconnect::model::ast::keyword::Direction;
use std::sync::Arc;

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::Program;
use fhirconnect::resolve::program::mapping::Method;
use fhirconnect::resolve::program::target::Target;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::TemplateSource;

use crate::resolve::CONTEXT;
use crate::resolve::EXTENSION;
use crate::resolve::PROBLEM;
use crate::resolve::borrow;
use crate::resolve::chain;
use crate::resolve::codes;
use crate::resolve::context;
use crate::resolve::model;
use crate::resolve::program::twinned_template;
use crate::resolve::program_of;
use crate::resolve::refusals;
use crate::resolve::render;
use crate::resolve::set_of;
use crate::resolve::start_context;
use crate::resolve::start_model;

#[test]
fn an_archetype_the_slot_does_not_carry_is_refused_naming_both() -> Result<(), Box<dyn Error>> {
    let slot = "mappings:\n  - name: \"slot\"\n    with:\n      fhir: \"$resource\"\n      openehr: \"$archetype/data[at0001]/items[openEHR-EHR-CLUSTER.problem_qualifier.v2]\"\n    slotArchetype: \"CLUSTER.other.v1\"\n";
    let files = [
        ("model.yml", start_model(slot)),
        (
            "cluster.yml",
            model(
                "CLUSTER.other.v1",
                "openEHR-EHR-CLUSTER.anatomical_location.v1",
                "mappings: []\n",
            ),
        ),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-archetype-mismatch"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(
        message.contains("openEHR-EHR-CLUSTER.anatomical_location.v1"),
        "{message}"
    );
    assert!(
        message.contains("openEHR-EHR-CLUSTER.problem_qualifier.v2"),
        "{message}"
    );
    Ok(())
}

#[test]
fn a_slot_cycle_is_refused_naming_the_chain() -> Result<(), Box<dyn Error>> {
    let slot = "mappings:\n  - name: \"slot\"\n    with:\n      fhir: \"$resource\"\n      openehr: \"$archetype/data[at0001]/items[openEHR-EHR-CLUSTER.problem_qualifier.v2]\"\n    slotArchetype: \"CLUSTER.looping.v2\"\n";
    let inner = "mappings:\n  - name: \"back\"\n    with:\n      fhir: \"$resource\"\n      openehr: \"$archetype\"\n    slotArchetype: \"EVALUATION.synthetic.v1\"\n";
    let files = [
        ("model.yml", start_model(slot)),
        (
            "cluster.yml",
            model(
                "CLUSTER.looping.v2",
                "openEHR-EHR-CLUSTER.problem_qualifier.v2",
                inner,
            ),
        ),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-slot-cycle"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(
        message.contains("EVALUATION.synthetic.v1 -> CLUSTER.looping.v2"),
        "{message}"
    );
    Ok(())
}

#[test]
fn a_caret_above_the_resource_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"^.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unanchored-fhir-path"]);
    Ok(())
}

#[test]
fn a_parent_step_above_the_archetype_root_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/../../../../..\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["path-above-anchor-root"]);
    Ok(())
}

#[test]
fn a_read_only_expression_on_a_write_side_is_refused_naming_the_step() -> Result<(), Box<dyn Error>>
{
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.category.first().coding\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-read-only-fhir-write"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("first()"), "{message}");
    Ok(())
}

#[test]
fn a_read_only_expression_is_accepted_where_only_fhir_is_read() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    unidirectional: \"fhir->openehr\"\n    with:\n      fhir: \"$resource.category.first().coding\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
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
    assert_eq!(mapping.direction(), Some(Direction::FhirToOpenehr));
    Ok(())
}

/// "The condition is applied on the input data" and §targetAttribute itself
/// writes `$resource.identifier.where(type.coding.code="room")`
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`), so a
/// condition path is read and a step that filters belongs there.
#[test]
fn a_filtering_expression_in_a_condition_is_accepted() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    fhirCondition:\n      targetRoot: \"$resource.identifier.where(type.coding.code = 'room')\"\n      targetAttribute: \"value\"\n      operator: \"not empty\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
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
    let Target::Fhir(ref root) = *condition.target() else {
        return Err("the condition filters the FHIR side".into());
    };
    assert_eq!(
        root.expression().as_str(),
        "$resource.identifier.where(type.coding.code = 'room')"
    );
    assert!(matches!(
        *root.writability(),
        fhirconnect::tree::path::Writability::ReadOnly { .. }
    ));
    assert_eq!(condition.attributes().len(), 1);
    Ok(())
}

/// The same holds for an ordinal step: a condition never writes, so `first()`
/// in one is resolved rather than refused
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`).
#[test]
fn an_ordinal_step_in_a_condition_is_accepted() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    fhirCondition:\n      targetRoot: \"$resource.category.first()\"\n      targetAttribute: \"coding.code\"\n      operator: \"one of\"\n      criteria:\n        - \"encounter-diagnosis\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
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
    let Target::Fhir(ref root) = *condition.target() else {
        return Err("the condition filters the FHIR side".into());
    };
    assert_eq!(root.expression().as_str(), "$resource.category.first()");
    let Some(Target::Fhir(attribute)) = condition.attributes().first() else {
        return Err("the condition names one attribute".into());
    };
    assert_eq!(
        attribute.expression().as_str(),
        "$resource.category.first().coding.code"
    );
    Ok(())
}

#[test]
fn a_fhir_expression_outside_the_grammar_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.category.where(coding.code = 'x'\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-malformed-fhir-path"]);
    Ok(())
}

#[test]
fn an_openehr_path_outside_the_grammar_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-malformed-openehr-path"]);
    Ok(())
}

/// `$resource` is a FHIR-side variable
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`), so an
/// openEHR path opening with it binds to no openEHR anchor.
#[test]
fn an_openehr_path_opening_with_a_fhir_variable_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$resource/data[at0001]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unbound-path-variable"]);
    Ok(())
}

/// The worked `^^` example maps an Encounter diagnosis, whose type "is
/// contained in the Encounter, while the rest of the diagnosis is contained in
/// a referenced `Condition`", and reaches `diagnosis.use` from inside that
/// reference
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/path_operators.adoc`,
/// §Recurrence and parent elements).
#[test]
fn a_caret_run_climbs_out_of_a_referenced_resource() -> Result<(), Box<dyn Error>> {
    let encounter = String::from(
        "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: EVALUATION.synthetic.v1\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  openEhrConfig:\n    archetype: openEHR-EHR-EVALUATION.problem_diagnosis.v1\n  fhirConfig:\n    structureDefinition: http://hl7.org/fhir/StructureDefinition/Encounter\nmappings:\n  - name: \"problemDiagnosis\"\n    with:\n      fhir: \"$resource.diagnosis\"\n      openehr: \"$archetype\"\n      type: \"NONE\"\n    followedBy:\n      mappings:\n        - name: \"referencedDiagnose\"\n          with:\n            fhir: \"condition.reference\"\n            openehr: \"$reference\"\n          reference:\n            resourceType: \"Condition\"\n            mappings:\n              - name: \"diagnosisTyp\"\n                with:\n                  fhir: \"^^.use.coding\"\n                  openehr: \"data[at0001]/items[at0002]\"\n",
    );
    let files = [
        ("model.yml", encounter),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let diagnosis = program.mappings().first().ok_or("the root mapping")?;
    let referenced = diagnosis.followed_by().first().ok_or("the reference")?;
    let Method::Reference { ref mappings, .. } = *referenced.method() else {
        return Err(format!("the mapping compiled to {:?}", referenced.method()).into());
    };
    let inside = mappings.first().ok_or("the mapping inside the reference")?;
    let target = inside.fhir().ok_or("the FHIR side")?;
    assert_eq!(
        target.expression().as_str(),
        "$resource.diagnosis.use.coding"
    );
    Ok(())
}

#[test]
fn an_openehr_path_naming_no_node_of_the_template_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at9999]\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-template-node"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("at9999"), "{message}");
    Ok(())
}

/// A path that reaches two nodes binds to neither: picking one would map a
/// clinical value into a node nothing named, and shortening the path to their
/// parent would lose the occurrence axis.
#[test]
fn an_openehr_path_naming_two_template_nodes_is_refused_naming_both() -> Result<(), Box<dyn Error>>
{
    let body = "mappings:\n  - name: \"contextStartTime\"\n    with:\n      fhir: \"$resource.recordedDate\"\n      openehr: \"$composition/context/start_time\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let index = twinned_template("start_time")?;
    let diagnostics = compile(
        &set,
        &MappingName::new("synthetic.context")?,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .err()
    .ok_or("two nodes at one path is a refusal")?;
    assert_eq!(codes(&diagnostics), vec!["fc-ambiguous-template-node"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("names 2 nodes"), "{message}");
    assert!(message.contains("_twin"), "{message}");
    Ok(())
}

/// The same refusal for a path the Web Template compacted: the builder folds
/// an `ELEMENT` into its value node, so two `ELEMENT`s of one at-code share the
/// one shortened path a mapping writes.
#[test]
fn a_compacted_openehr_path_naming_two_template_nodes_is_refused() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let index = twinned_template("at0002")?;
    let diagnostics = compile(
        &set,
        &MappingName::new("synthetic.context")?,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .err()
    .ok_or("two compacted nodes at one path is a refusal")?;
    assert_eq!(codes(&diagnostics), vec!["fc-ambiguous-template-node"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("names 2 nodes"), "{message}");
    Ok(())
}

/// An openEHR path may reach past the deepest template node into the reference
/// model, and what it walks there is checked against the RM attribute model
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html>).
#[test]
fn an_rm_tail_the_reference_model_does_not_define_is_refused() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/not_an_attribute\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-template-node"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(
        message.contains("`not_an_attribute` is no attribute of `EVALUATION`"),
        "{message}"
    );
    Ok(())
}

/// A reference-model attribute path below the node compiles when a FLAT part
/// of the node's class carries it.
#[test]
fn an_rm_tail_the_reference_model_defines_compiles() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"certaintyCode\"\n    with:\n      fhir: \"$resource.verificationStatus.coding.code\"\n      openehr: \"$archetype/data[at0001]/items[at0073]/defining_code/code_string\"\n";
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
    let openehr = program
        .mappings()
        .first()
        .ok_or("the compiled mapping")?
        .openehr()
        .ok_or("its openEHR side")?;
    assert_eq!(openehr.tail().to_string(), "defining_code/code_string");
    Ok(())
}

/// A tail the reference model defines below a structural node, through a
/// list, is refused at load: the engine writes a tail as one value of the
/// node, so its value would never reach the wire.
#[test]
fn an_rm_tail_no_flat_part_carries_is_refused_at_load() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"provider\"\n    with:\n      fhir: \"$resource.recorder\"\n      openehr: \"$archetype/other_participations/function\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-uncarried-tail"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(
        message.contains("`provider`")
            && message.contains("`other_participations/function`")
            && message.contains("EVALUATION"),
        "the refusal names the mapping, the tail and the class: {message}"
    );
    Ok(())
}

/// A `manual` path that ends on no string attribute is refused at load, as a
/// `with.openehr` tail the engine cannot carry is: `merge` writes a manual
/// value as text, and `defining_code/terminology_id` holds a `TERMINOLOGY_ID`.
#[test]
fn a_manual_path_no_flat_part_carries_is_refused_at_load() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"certainty\"\n    with:\n      fhir: \"$resource.verificationStatus\"\n      openehr: \"$archetype/data[at0001]/items[at0073]\"\n    manual:\n      - name: \"linked\"\n        openehr:\n          - path: \"defining_code/terminology_id\"\n            value: \"local\"\n";
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-uncarried-tail"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(
        message.contains("`certainty.linked`")
            && message.contains("`defining_code/terminology_id`"),
        "{message}"
    );
    Ok(())
}

/// The published cluster anchors on `$resource`, which is "Path of the root
/// resource" (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`),
/// so its condition reads `Condition.coding`, an element FHIR R4 does not
/// define (<https://hl7.org/fhir/R4/condition.html>). Reported as #173. The
/// two warnings beside the refusals are `problem_diagnosis.v1.yml` lines 73
/// and 100, mappings with no `type` onto a structural node and no child, and
/// the three `fc-uncarried-tail` rows are its lines 45, 47 and 100, tails no
/// FLAT part of `EVENT_CONTEXT` or `EVALUATION` carries.
#[test]
fn the_published_anatomical_location_slot_is_refused_naming_the_element()
-> Result<(), Box<dyn Error>> {
    let set = chain(&[
        CONTEXT,
        EXTENSION,
        "contexts/ferrobridge_anatomical.context.yml",
    ])
    .map_err(|diagnostics| render(&diagnostics))?;
    let diagnostics = program_of(&set, "ferrobridge_anatomical.context")
        .err()
        .ok_or("the published cluster resolves under a Condition")?;
    assert_eq!(
        codes(&diagnostics),
        vec![
            "fc-anchor-without-children",
            "fc-anchor-without-children",
            "fc-uncarried-tail",
            "fc-uncarried-tail",
            "fc-uncarried-tail",
            "fc-unknown-fhir-element",
            "fc-unknown-template-node",
            "fc-unknown-template-node"
        ]
    );
    let named = diagnostics
        .iter()
        .map(Diagnostic::message)
        .find(|message| message.contains("`Condition` defines no element `coding`"));
    assert!(named.is_some(), "{}", render(&diagnostics));
    Ok(())
}

/// The published `other_participations` method writes
/// `$archetype/diagnose/other_participations`, and the openEHR reference model
/// gives `EVALUATION` no `diagnose` attribute
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html>).
/// Recorded on #101; the context this crate ships overwrites the method.
#[test]
fn the_published_evaluation_tail_outside_the_reference_model_is_refused()
-> Result<(), Box<dyn Error>> {
    let set = chain(&[
        CONTEXT,
        EXTENSION,
        "contexts/ferrobridge_anatomical.context.yml",
    ])
    .map_err(|diagnostics| render(&diagnostics))?;
    let diagnostics = program_of(&set, "ferrobridge_anatomical.context")
        .err()
        .ok_or("the published tail walks outside the reference model")?;
    let message = diagnostics
        .iter()
        .map(Diagnostic::message)
        .find(|message| message.contains("diagnose"))
        .ok_or("a refusal naming the tail")?;
    assert!(
        message.contains("`diagnose` is no attribute of `EVALUATION`"),
        "{message}"
    );
    Ok(())
}

/// The context body of the renaming cases, over the published `KDS_Diagnose`
/// template, which names the elements it constrains.
fn renamed_context() -> String {
    context(
        "synthetic.context",
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/synthetic\"\n  \
         template:\n    id: \"KDS_Diagnose\"\n  archetypes:\n    - \
         \"EVALUATION.synthetic.v1\"\n  start: \"EVALUATION.synthetic.v1\"\n",
    )
}

/// Compiles one synthetic start model against the published `KDS_Diagnose`.
fn against_kds(body: &str) -> Result<Result<Arc<Program>, Vec<Diagnostic>>, Box<dyn Error>> {
    let set = set_of(&[
        ("model.yml", start_model(body).as_str()),
        ("context.yml", renamed_context().as_str()),
    ])?;
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::KDS_DIAGNOSE_OPT)?;
    let index = WebTemplateIndex::build(&TemplateSource::Opt14(Box::new(opt)))?;
    Ok(compile(
        &set,
        &MappingName::new("synthetic.context")?,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    ))
}

/// A template may constrain an element's name, and its `aqlPath` then carries
/// the name as a predicate (`[at0002,'Kodierte Diagnose']`); a mapping names
/// the node by its node id, and a name it does not write selects nothing, the
/// rule the index applies to an indexed node. The second element sits below an
/// archetype root the template renames too. No specification governs this:
/// our own design.
#[test]
fn a_path_that_writes_no_name_resolves_to_the_element_the_template_names()
-> Result<(), Box<dyn Error>> {
    let program = against_kds(
        "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      \
         openehr: \"$archetype/data[at0001]/items[at0002]\"\n  - name: \"clinicalStatus\"\n    \
         with:\n      fhir: \"$resource.clinicalStatus\"\n      openehr: \
         \"$archetype/data[at0001]/items[openEHR-EHR-CLUSTER.problem_qualifier.v2]/items[at0003]\"\n",
    )?
    .map_err(|diagnostics| render(&diagnostics))?;
    let text = program.to_string();
    assert!(
        text.contains("items[at0002,'Kodierte Diagnose']/value DV_CODED_TEXT"),
        "{text}"
    );
    assert!(
        text.contains(
            "items[openEHR-EHR-CLUSTER.problem_qualifier.v2,'Klinischer Status']/items[at0003,'Klinischer Status']/value DV_CODED_TEXT"
        ),
        "{text}"
    );
    Ok(())
}

/// A name the mapping does write must be the name the template carries.
#[test]
fn a_path_whose_name_the_template_does_not_carry_is_refused() -> Result<(), Box<dyn Error>> {
    let diagnostics = against_kds(
        "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      \
         openehr: \"$archetype/data[at0001]/items[at0002,'Another name']\"\n",
    )?
    .err()
    .ok_or("a name the template does not carry compiled")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-template-node"]);
    Ok(())
}

/// The two fixtures that name a tail no FLAT part carries, one through
/// `with.openehr` and one through a `manual` path, are refused at load.
#[test]
fn the_uncarried_tail_fixtures_are_refused_at_load() -> Result<(), Box<dyn Error>> {
    for fixture in [
        "ferrobridge_tail_unsupported",
        "ferrobridge_manual_unsupported",
    ] {
        let error = crate::support::compiled(fixture)
            .err()
            .ok_or_else(|| format!("{fixture} compiled"))?;
        assert!(
            error.to_string().contains("fc-uncarried-tail"),
            "{fixture}: {error}"
        );
    }
    Ok(())
}
