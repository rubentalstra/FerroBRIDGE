// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The diagnosis chain compiled into one program, the determinism of the
//! compile and the selection among programs.

use core::error::Error;
use std::sync::Arc;

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::model::load::MappingSet;
use fhirconnect::model::load::load_context_file;
use fhirconnect::model::load::load_model_file;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::Program;
use fhirconnect::resolve::program::binding::Pin;
use fhirconnect::resolve::program::binding::TemplateId;
use fhirconnect::resolve::program::mapping::Mapping;
use fhirconnect::resolve::select::SelectError;
use fhirconnect::resolve::select::select_by_profile;
use fhirconnect::resolve::select::select_by_profile_pinned;
use fhirconnect::resolve::select::select_by_template;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::Generation;
use openehr_mapping_core::template::TemplateSource;
use openehr_mapping_core::template::web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplateNode;
use proptest::prelude::ProptestConfig;
use proptest::proptest;
use proptest::test_runner::TestCaseError;

use crate::resolve::CONTEXT;
use crate::resolve::CORPUS;
use crate::resolve::EXTENSION;
use crate::resolve::FIXTURES;
use crate::resolve::PROBLEM;
use crate::resolve::PUBLISHED;
use crate::resolve::STAND_INS;
use crate::resolve::borrow;
use crate::resolve::chain;
use crate::resolve::context;
use crate::resolve::program_of;
use crate::resolve::render;
use crate::resolve::set_of;
use crate::resolve::start_context;
use crate::resolve::start_model;
use crate::resolve::template;

#[test]
fn the_diagnosis_chain_compiles_into_one_program() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let program = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    insta::assert_snapshot!("diagnosis_program", program.to_string());
    Ok(())
}

#[test]
fn the_compiled_program_records_what_it_was_compiled_against() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let program = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    assert_eq!(program.resource().as_str(), "Condition");
    assert_eq!(
        program.template().id().as_str(),
        ferrobridge_testkit::fixtures::DIAGNOSE_TEMPLATE_ID
    );
    assert_eq!(
        program.profile().version(),
        &Pin::Pinned(String::from("1.0.0"))
    );
    let models: Vec<&str> = program
        .models()
        .iter()
        .map(|model| model.name().as_str())
        .collect();
    assert_eq!(
        models,
        vec![
            "EVALUATION.problem_diagnosis.v1",
            "CLUSTER.problem_qualifier.v2"
        ]
    );
    Ok(())
}

#[test]
fn compiling_the_same_set_twice_yields_the_same_program() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let first = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    let second = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    assert_eq!(first, second);
    assert_eq!(first.to_string(), second.to_string());
    Ok(())
}

/// Inserts each file into one set in the order given.
///
/// `load_set` sorts the paths it is handed, so a permutation of its input
/// never reaches the compiler. This is the seam that can genuinely see an
/// order: the registry the compiler reads.
fn inserted(paths: &[String]) -> Result<MappingSet, String> {
    let mut set = MappingSet::new();
    for path in paths {
        if path.ends_with(".context.yml") {
            let file = load_context_file(path).map_err(|diagnostics| render(&diagnostics))?;
            set.insert_context(file)
                .map_err(|error| error.to_string())?;
        } else {
            let file = load_model_file(path).map_err(|diagnostics| render(&diagnostics))?;
            set.insert_model(file).map_err(|error| error.to_string())?;
        }
    }
    Ok(set)
}

/// The files of the diagnosis chain, in the order they are written here.
fn chain_paths() -> Vec<String> {
    PUBLISHED
        .iter()
        .map(|file| format!("{CORPUS}/{file}"))
        .chain(
            STAND_INS
                .iter()
                .chain([CONTEXT, EXTENSION].iter())
                .map(|file| format!("{FIXTURES}/{file}")),
        )
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    /// The order the files enter the set never changes the program.
    #[test]
    fn the_program_does_not_depend_on_the_order_the_files_load(
        seed in proptest::collection::vec(0_usize..64, 6),
    ) {
        let mut files = chain_paths();
        let mut keyed: Vec<(usize, String)> = seed
            .iter()
            .copied()
            .chain(core::iter::repeat(0))
            .zip(files.drain(..))
            .collect();
        keyed.sort_by_key(|(key, _)| *key);
        let shuffled: Vec<String> = keyed.into_iter().map(|(_, file)| file).collect();
        let set = inserted(&shuffled).map_err(TestCaseError::fail)?;
        let program = program_of(&set, "ferrobridge_diagnose.context")
            .map_err(|diagnostics| TestCaseError::fail(render(&diagnostics)))?;
        let expected = {
            let set = inserted(&chain_paths()).map_err(TestCaseError::fail)?;
            program_of(&set, "ferrobridge_diagnose.context")
                .map_err(|diagnostics| TestCaseError::fail(render(&diagnostics)))?
        };
        proptest::prop_assert_eq!(program.to_string(), expected.to_string());
    }
}

/// Builds the diagnosis template with a semantic version it does not carry.
///
/// The ADL 1.4 route serves an operational template with no semantic version,
/// so the pair of versions the `sem_ver` selector compares only exists for a
/// template served over the ADL 2 route.
pub(super) fn versioned_template(sem_ver: &str) -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::DIAGNOSE_OPT)?;
    let mut built = web_template(&TemplateSource::Opt14(Box::new(opt)))?;
    built.sem_ver = Some(String::from(sem_ver));
    Ok(WebTemplateIndex::over(built, Generation::Adl2)?)
}

/// Builds the diagnosis template with one archetype identifier in full form.
///
/// A Web Template built over the ADL 1.4 route carries the interface form of
/// an archetype identifier, and only the ADL 2 full form states a release
/// version, so the pair of values the `revision` selector compares exists for
/// an ADL 2 template alone.
pub(super) fn released_template(
    archetype: &str,
    full: &str,
) -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::DIAGNOSE_OPT)?;
    let mut built = web_template(&TemplateSource::Opt14(Box::new(opt)))?;
    rename_node(&mut built.tree, archetype, full);
    Ok(WebTemplateIndex::over(built, Generation::Adl2)?)
}

/// Rewrites the node id of every node of a tree that carries `archetype`.
fn rename_node(node: &mut WebTemplateNode, archetype: &str, full: &str) {
    if node.node_id.as_deref() == Some(archetype) {
        node.node_id = Some(String::from(full));
    }
    for child in &mut node.children {
        rename_node(child, archetype, full);
    }
}

/// Builds the diagnosis template with a second node beside `node_id`.
///
/// Two nodes of one archetype under one parent carry one `aqlPath` and are
/// told apart by the runtime name the template fixes, which is the shape an
/// openEHR path with no name predicate cannot resolve to one node
/// (<https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/>).
pub(super) fn twinned_template(id: &str) -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::DIAGNOSE_OPT)?;
    let mut built = web_template(&TemplateSource::Opt14(Box::new(opt)))?;
    assert!(twin_node(&mut built.tree, id), "the template carries {id}");
    Ok(WebTemplateIndex::over(built, Generation::Adl14)?)
}

/// Inserts a clone of the child whose `id` or `nodeId` is `id` beside it.
fn twin_node(node: &mut WebTemplateNode, id: &str) -> bool {
    let found = node
        .children
        .iter()
        .position(|child| child.id == id || child.node_id.as_deref() == Some(id));
    if let Some(at) = found {
        let Some(child) = node.children.get(at) else {
            return false;
        };
        let mut twin = child.clone();
        twin.id = format!("{}_twin", twin.id);
        twin.name = Some(String::from("Second"));
        node.children.insert(at.saturating_add(1), twin);
        return true;
    }
    node.children.iter_mut().any(|child| twin_node(child, id))
}

/// Builds the diagnosis template under another identifier.
fn renamed_template(id: &str) -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::DIAGNOSE_OPT)?;
    let mut built = web_template(&TemplateSource::Opt14(Box::new(opt)))?;
    built.template_id = String::from(id);
    Ok(WebTemplateIndex::over(built, Generation::Adl14)?)
}

/// Compiles two programs that claim one profile and two templates.
fn two_programs() -> Result<Vec<Arc<Program>>, Box<dyn Error>> {
    let model = start_model(PROBLEM);
    let first = context("first.context", &start_context(&[]));
    let second = context(
        "second.context",
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/synthetic\"\n  template:\n    id: \"another.template.v1\"\n  archetypes:\n    - \"EVALUATION.synthetic.v1\"\n  start: \"EVALUATION.synthetic.v1\"\n",
    );
    let set = set_of(&[
        ("model.yml", model.as_str()),
        ("first.yml", first.as_str()),
        ("second.yml", second.as_str()),
    ])?;
    let here = template()?;
    let there = renamed_template("another.template.v1")?;
    let codes = StaticMappingCodes::default();
    let programs = vec![
        compile(
            &set,
            &MappingName::new("first.context")?,
            &here,
            &SCHEMAS,
            &codes,
        )
        .map_err(|diagnostics| render(&diagnostics))?,
        compile(
            &set,
            &MappingName::new("second.context")?,
            &there,
            &SCHEMAS,
            &codes,
        )
        .map_err(|diagnostics| render(&diagnostics))?,
    ];
    Ok(programs)
}

/// The engine needs to know whether a FHIR target carries many values, the
/// counterpart of the openEHR occurrence axes.
#[test]
fn a_fhir_target_says_whether_it_repeats() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.category\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n    followedBy:\n      mappings:\n        - name: \"one\"\n          unidirectional: \"fhir->openehr\"\n          with:\n            fhir: \"$resource.category.first()\"\n            openehr: \"$archetype/data[at0001]/items[at0002]\"\n        - name: \"scalar\"\n          with:\n            fhir: \"$resource.recordedDate\"\n            openehr: \"$archetype/data[at0001]/items[at0002]\"\n";
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
    let repeating = program.mappings().first().ok_or("one mapping")?;
    assert!(repeating.fhir().ok_or("the FHIR side")?.repeats());
    let narrowed = repeating.followed_by().first().ok_or("the first child")?;
    assert!(!narrowed.fhir().ok_or("the FHIR side")?.repeats());
    let scalar = repeating.followed_by().get(1).ok_or("the second child")?;
    assert!(!scalar.fhir().ok_or("the FHIR side")?.repeats());
    Ok(())
}

/// A method is addressed by its dotted name, so the program looks one up
/// without the engine walking the tree
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Append).
#[test]
fn the_program_looks_up_a_mapping_by_its_dotted_name() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let program = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    let found = program
        .mapping_named("problemDiagnose")
        .ok_or("the start method")?;
    assert_eq!(found.name(), "problemDiagnose");
    let nested = program
        .mappings()
        .iter()
        .find_map(|mapping| mapping.followed_by().first())
        .ok_or("a nested method")?;
    assert_eq!(
        program.mapping_named(nested.name()).map(Mapping::name),
        Some(nested.name())
    );
    assert!(program.mapping_named("noSuchMethod").is_none());
    Ok(())
}

#[test]
fn two_programs_sharing_a_profile_are_refused_without_a_pin() -> Result<(), Box<dyn Error>> {
    let programs = two_programs()?;
    let claimed = vec![String::from(
        "http://example.org/fhir/StructureDefinition/synthetic",
    )];
    let error = select_by_profile(&programs, &claimed)
        .err()
        .ok_or("two programs share the profile")?;
    match error {
        SelectError::Ambiguous { ref candidates, .. } => {
            assert_eq!(candidates.len(), 2, "{candidates:?}");
        }
        other => return Err(format!("{other}").into()),
    }
    let pin = TemplateId::new("another.template.v1");
    let selected = select_by_profile_pinned(&programs, &claimed, Some(&pin))?;
    assert_eq!(selected.template().id(), &pin);
    Ok(())
}

#[test]
fn a_program_is_selected_by_the_template_a_composition_names() -> Result<(), Box<dyn Error>> {
    let programs = two_programs()?;
    let wanted = TemplateId::new(ferrobridge_testkit::fixtures::DIAGNOSE_TEMPLATE_ID);
    let selected = select_by_template(&programs, &wanted)?;
    assert_eq!(selected.template().id(), &wanted);
    let absent = TemplateId::new("no.such.template.v1");
    assert!(matches!(
        select_by_template(&programs, &absent),
        Err(SelectError::NoMatch { .. })
    ));
    Ok(())
}
