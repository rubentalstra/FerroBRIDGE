// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Context resolution: one immutable program per context.
//!
//! The chain under test is the diagnosis chain of the vendored FHIRconnect
//! mapping library, compiled against the synthetic operational template of the
//! testkit through a FerroBRIDGE-authored context. The ordering, collision and
//! version rules the specification leaves open each get their own isolated
//! case over small synthetic files written in the test.

use core::error::Error;
use fhirconnect::model::ast::Direction;
use std::sync::Arc;

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::model::ast::ContextMappingFile;
use fhirconnect::model::ast::ModelMappingFile;
use fhirconnect::model::load::MappingSet;
use fhirconnect::model::load::load_set;
use fhirconnect::model::parse::lower_context;
use fhirconnect::model::parse::lower_model;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::Pin;
use fhirconnect::resolve::program::Program;
use fhirconnect::resolve::program::TemplateId;
use fhirconnect::resolve::select::SelectError;
use fhirconnect::resolve::select::select_by_profile;
use fhirconnect::resolve::select::select_by_profile_pinned;
use fhirconnect::resolve::select::select_by_template;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::loader::load_str;
use openehr_mapping_core::template::Generation;
use openehr_mapping_core::template::TemplateSource;
use openehr_mapping_core::template::web_template;
use proptest::prelude::ProptestConfig;
use proptest::proptest;
use proptest::test_runner::TestCaseError;

/// The vendored mapping library, relative to this crate's manifest.
const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/fhirconnect-mapping-lib"
);

/// The fixtures this crate's tests ship.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// The published files of the diagnosis chain that load on their own.
const PUBLISHED: &[&str] = &[
    "model/evaluation/org.openehr/problem_diagnosis.v1.yml",
    "model/cluster/org.openehr/anatomical_location.v1.yml",
    "model/cluster/org.openehr/problem_qualifier.v2.yml",
    "projects/org.highmed/KDS/diagnose/KDS_problem_qualifier.yml",
];

/// The context this crate authored over that chain.
const CONTEXT: &str = "contexts/ferrobridge_diagnose.context.yml";

/// The extension this crate authored over that chain.
const EXTENSION: &str = "extensions/ferrobridge_problem_diagnose.yml";

/// Builds the index over the testkit's diagnosis template.
fn template() -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::DIAGNOSE_OPT)?;
    Ok(WebTemplateIndex::build(&TemplateSource::Opt14(Box::new(
        opt,
    )))?)
}

/// Loads the published chain plus the fixtures this test names.
fn chain(fixtures: &[&str]) -> Result<MappingSet, Vec<Diagnostic>> {
    let files: Vec<String> = PUBLISHED
        .iter()
        .map(|file| format!("{CORPUS}/{file}"))
        .chain(fixtures.iter().map(|file| format!("{FIXTURES}/{file}")))
        .collect();
    load_set(files, &StaticMappingCodes::default())
}

/// Builds a set from in-memory files, for the rules that need small inputs.
fn set_of(files: &[(&str, &str)]) -> Result<MappingSet, Box<dyn Error>> {
    let mut set = MappingSet::new();
    for (name, text) in files {
        let document = load_str(*name, text)?;
        if text.contains("\ntype: context\n") {
            let file: ContextMappingFile =
                lower_context(&document).map_err(|diagnostics| render(&diagnostics))?;
            set.insert_context(file)
                .map_err(|error| error.to_string())?;
        } else {
            let file: ModelMappingFile =
                lower_model(&document).map_err(|diagnostics| render(&diagnostics))?;
            set.insert_model(file).map_err(|error| error.to_string())?;
        }
    }
    Ok(set)
}

/// Renders a diagnostic list as one error message.
fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{} {:?} {}: {}",
                diagnostic.file().display(),
                diagnostic.position(),
                diagnostic.code(),
                diagnostic.message()
            )
        })
        .collect::<Vec<String>>()
        .join("; ")
}

/// Returns the code and message of every diagnostic, sorted.
fn codes(diagnostics: &[Diagnostic]) -> Vec<String> {
    let mut found: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code().to_string())
        .collect();
    found.sort();
    found
}

/// Compiles one context out of a set against the diagnosis template.
fn program_of(set: &MappingSet, context: &str) -> Result<Arc<Program>, Vec<Diagnostic>> {
    let index = template().map_err(|error| {
        vec![Diagnostic::error(
            "tests",
            openehr_mapping_core::diagnostic::DiagnosticCode::TemplateBuild,
            error.to_string(),
        )]
    })?;
    let name = MappingName::new(context).map_err(|error| {
        vec![Diagnostic::error(
            "tests",
            openehr_mapping_core::diagnostic::DiagnosticCode::InvalidMappingName,
            error.to_string(),
        )]
    })?;
    compile(set, &name, &index, &SCHEMAS, &StaticMappingCodes::default())
}

/// The header every small synthetic model file in this test opens with.
fn model(name: &str, archetype: &str, body: &str) -> String {
    format!(
        "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: {name}\n  version: \
         1.0.0\nspec:\n  system: FHIR\n  version: R4\n  openEhrConfig:\n    archetype: \
         {archetype}\n  fhirConfig:\n    structureDefinition: \
         http://hl7.org/fhir/StructureDefinition/Condition\n{body}"
    )
}

/// The header every small synthetic extension file in this test opens with.
fn extension(name: &str, extends: &str, body: &str) -> String {
    format!(
        "grammar: FHIRConnect/v1.0.0\ntype: extension\nmetadata:\n  name: {name}\n  version: \
         1.0.0\nspec:\n  system: FHIR\n  version: R4\n  extends: {extends}\n{body}"
    )
}

/// The header every small synthetic context file in this test opens with.
fn context(name: &str, body: &str) -> String {
    format!(
        "grammar: FHIRConnect/v1.0.0\ntype: context\nmetadata:\n  name: {name}\n  version: \
         1.0.0\nspec:\n  system: FHIR\n  version: R4\ncontext:\n{body}"
    )
}

/// The start model every small synthetic case in this test compiles.
fn start_model(body: &str) -> String {
    model(
        "EVALUATION.synthetic.v1",
        "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
        body,
    )
}

/// The context body every small synthetic case in this test compiles.
fn start_context(extensions: &[&str]) -> String {
    let listed = if extensions.is_empty() {
        String::new()
    } else {
        let mut names = String::from("  extensions:\n");
        for name in extensions {
            names.push_str("    - \"");
            names.push_str(name);
            names.push_str("\"\n");
        }
        names
    };
    format!(
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/synthetic\"\n  \
         template:\n    id: \"ferrobridge.diagnose.v1\"\n  archetypes:\n    - \
         \"EVALUATION.synthetic.v1\"\n{listed}  start: \"EVALUATION.synthetic.v1\"\n"
    )
}

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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    /// The file order the loader is handed never changes the program.
    #[test]
    fn the_program_does_not_depend_on_the_order_the_files_load(
        seed in proptest::collection::vec(0_usize..64, 6),
    ) {
        let mut files: Vec<String> = PUBLISHED
            .iter()
            .map(|file| format!("{CORPUS}/{file}"))
            .chain([CONTEXT, EXTENSION].iter().map(|file| format!("{FIXTURES}/{file}")))
            .collect();
        let mut keyed: Vec<(usize, String)> = seed
            .iter()
            .copied()
            .chain(core::iter::repeat(0))
            .zip(files.drain(..))
            .collect();
        keyed.sort_by_key(|(key, _)| *key);
        let shuffled: Vec<String> = keyed.into_iter().map(|(_, file)| file).collect();
        let set = load_set(shuffled, &StaticMappingCodes::default())
            .map_err(|diagnostics| TestCaseError::fail(render(&diagnostics)))?;
        let program = program_of(&set, "ferrobridge_diagnose.context")
            .map_err(|diagnostics| TestCaseError::fail(render(&diagnostics)))?;
        let expected = {
            let set = load_set(
                PUBLISHED
                    .iter()
                    .map(|file| format!("{CORPUS}/{file}"))
                    .chain([CONTEXT, EXTENSION].iter().map(|file| format!("{FIXTURES}/{file}")))
                    .collect::<Vec<String>>(),
                &StaticMappingCodes::default(),
            )
            .map_err(|diagnostics| TestCaseError::fail(render(&diagnostics)))?;
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
fn versioned_template(sem_ver: &str) -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::DIAGNOSE_OPT)?;
    let mut built = web_template(&TemplateSource::Opt14(Box::new(opt)))?;
    built.sem_ver = Some(String::from(sem_ver));
    Ok(WebTemplateIndex::over(built, Generation::Adl2)?)
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
/// The one mapping method the small synthetic model files carry.
const PROBLEM: &str = "mappings:\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\
                       \n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n";

/// Borrows a list of named files for [`set_of`].
fn borrow<'a>(files: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    files
        .iter()
        .map(|(name, text)| (*name, text.as_str()))
        .collect()
}

/// Compiles a small synthetic case and returns its refusals.
fn refusals(files: &[(&str, &str)], context: &str) -> Result<Vec<Diagnostic>, Box<dyn Error>> {
    let set = set_of(files)?;
    match program_of(&set, context) {
        Ok(program) => Err(format!("the case compiled: {program}").into()),
        Err(diagnostics) => Ok(diagnostics),
    }
}

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

#[test]
fn a_template_the_context_does_not_name_is_refused_naming_both() -> Result<(), Box<dyn Error>> {
    let declaration = context(
        "synthetic.context",
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/synthetic\"\n  template:\n    id: \"another.template.v1\"\n  archetypes:\n    - \"EVALUATION.synthetic.v1\"\n  start: \"EVALUATION.synthetic.v1\"\n",
    );
    let files = [
        ("model.yml", start_model(PROBLEM)),
        ("context.yml", declaration),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-template-id-mismatch"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("another.template.v1"), "{message}");
    assert!(
        message.contains(ferrobridge_testkit::fixtures::DIAGNOSE_TEMPLATE_ID),
        "{message}"
    );
    Ok(())
}

#[test]
fn a_sem_ver_the_template_does_not_carry_is_recorded_unpinned() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let program = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    assert_eq!(program.template().sem_ver(), &Pin::Unpinned);
    assert!(program.template().sem_ver().is_unpinned());
    Ok(())
}

#[test]
fn a_sem_ver_disagreement_is_refused_naming_both_versions() -> Result<(), Box<dyn Error>> {
    let declaration = context(
        "synthetic.context",
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/synthetic\"\n  template:\n    id: \"ferrobridge.diagnose.v1\"\n    sem_ver: \"0.1.0\"\n  archetypes:\n    - \"EVALUATION.synthetic.v1\"\n  start: \"EVALUATION.synthetic.v1\"\n",
    );
    let files = [
        ("model.yml", start_model(PROBLEM)),
        ("context.yml", declaration),
    ];
    let set = set_of(&borrow(&files))?;
    let index = versioned_template("9.0.0-alpha.1")?;
    let name = MappingName::new("synthetic.context")?;
    let diagnostics = compile(
        &set,
        &name,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .err()
    .ok_or("a sem_ver disagreement is a refusal")?;
    assert_eq!(codes(&diagnostics), vec!["fc-template-sem-ver-mismatch"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("0.1.0"), "{message}");
    assert!(message.contains("9.0.0-alpha.1"), "{message}");
    Ok(())
}

#[test]
fn an_absent_profile_version_records_unpinned() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    assert_eq!(program.profile().version(), &Pin::Unpinned);
    Ok(())
}

#[test]
fn two_files_claiming_one_mapping_name_are_refused_naming_both() -> Result<(), Box<dyn Error>> {
    let first = start_model(PROBLEM);
    let second = start_model(PROBLEM);
    let error = set_of(&[
        ("first.yml", first.as_str()),
        ("second.yml", second.as_str()),
    ])
    .err()
    .ok_or("two files claiming one name are refused")?
    .to_string();
    assert!(error.contains("first.yml"), "{error}");
    assert!(error.contains("second.yml"), "{error}");
    Ok(())
}

#[test]
fn the_program_records_the_version_of_every_model_it_compiled() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let program = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    let start = program.models().first().ok_or("the start model")?;
    assert_eq!(start.version().as_str(), "0.0.1-alpha");
    assert_eq!(start.revision(), &Pin::Pinned(String::from("1.4.1")));
    assert_eq!(
        start.extensions(),
        [MappingName::new("ferrobridge_problem_diagnose")?]
    );
    Ok(())
}

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

#[test]
fn a_profile_version_the_program_does_not_pin_is_refused() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let programs = vec![
        program_of(&set, "ferrobridge_diagnose.context")
            .map_err(|diagnostics| render(&diagnostics))?,
    ];
    let claimed = vec![String::from(
        "http://example.org/fhir/StructureDefinition/ferrobridge-diagnosis|2.0.0",
    )];
    let error = select_by_profile(&programs, &claimed)
        .err()
        .ok_or("the instance claims another version")?;
    match error {
        SelectError::ProfileVersion { wanted, pinned } => {
            assert_eq!(wanted, "2.0.0");
            assert_eq!(pinned, "1.0.0");
        }
        other => return Err(format!("{other}").into()),
    }
    Ok(())
}
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
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-fhir-element"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(
        message.contains("`Condition` defines no element `coding`"),
        "{message}"
    );
    Ok(())
}
