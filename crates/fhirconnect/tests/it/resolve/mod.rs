// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Context resolution: one immutable program per context.
//!
//! The chain under test is the diagnosis chain of the vendored FHIRconnect
//! mapping library, compiled against the synthetic operational template of the
//! testkit through a FerroBRIDGE-authored context. The ordering, collision and
//! version rules the specification leaves open each get their own isolated
//! case over small synthetic files written in the test.

mod extensions;
mod methods;
mod paths;
mod program;
mod versions;

use core::error::Error;
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
use fhirconnect::resolve::program::Program;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::loader::load_str;
use openehr_mapping_core::template::TemplateSource;

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

/// The stand-ins every load of the chain carries beside the published files,
/// relative to the fixtures.
const STAND_INS: &[&str] =
    &["projects/ferrobridge/kds_diagnose/ferrobridge_kds_problem_qualifier.yml"];

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
        .chain(
            STAND_INS
                .iter()
                .chain(fixtures)
                .map(|file| format!("{FIXTURES}/{file}")),
        )
        .collect();
    load_set(files, &StaticMappingCodes::default())
}

/// Builds a set from in-memory files, for the rules that need small inputs.
pub(crate) fn set_of(files: &[(&str, &str)]) -> Result<MappingSet, Box<dyn Error>> {
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

/// Loads named in-memory files through the whole loader.
///
/// [`set_of`] lowers each file on its own, which skips the strict schema and
/// the semantic rules, so a case about either writes its files out and takes
/// the entry point a real load takes. The directory is returned because it
/// owns the files for as long as the test reads them.
fn loaded_set(files: &[(&str, &str)]) -> Result<(tempfile::TempDir, MappingSet), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let mut paths = Vec::new();
    for (name, text) in files {
        let path = directory.path().join(name);
        std::fs::write(&path, text)?;
        paths.push(path);
    }
    let set = load_set(paths, &StaticMappingCodes::default())
        .map_err(|diagnostics| render(&diagnostics))?;
    Ok((directory, set))
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
pub(crate) fn codes(diagnostics: &[Diagnostic]) -> Vec<String> {
    let mut found: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code().to_string())
        .collect();
    found.sort();
    found
}

/// Compiles one context out of a set against the diagnosis template.
pub(crate) fn program_of(set: &MappingSet, context: &str) -> Result<Arc<Program>, Vec<Diagnostic>> {
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
pub(crate) fn model(name: &str, archetype: &str, body: &str) -> String {
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
pub(crate) fn context(name: &str, body: &str) -> String {
    format!(
        "grammar: FHIRConnect/v1.0.0\ntype: context\nmetadata:\n  name: {name}\n  version: \
         1.0.0\nspec:\n  system: FHIR\n  version: R4\ncontext:\n{body}"
    )
}

/// The start model every small synthetic case in this test compiles.
pub(crate) fn start_model(body: &str) -> String {
    model(
        "EVALUATION.synthetic.v1",
        "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
        body,
    )
}

/// The start model, pinning an archetype revision.
fn revised_model(revision: &str, body: &str) -> String {
    format!(
        "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: \
         EVALUATION.synthetic.v1\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  \
         openEhrConfig:\n    archetype: openEHR-EHR-EVALUATION.problem_diagnosis.v1\n    \
         revision: \"{revision}\"\n  fhirConfig:\n    structureDefinition: \
         http://hl7.org/fhir/StructureDefinition/Condition\n{body}"
    )
}

/// The context body every small synthetic case in this test compiles.
pub(crate) fn start_context(extensions: &[&str]) -> String {
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

/// Compiles the diagnosis chain and returns it with the published files it
/// loads, relative to the vendored library.
pub(crate) fn diagnosis_chain() -> Result<crate::support::Chain, Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let program = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    Ok((PUBLISHED, program))
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
