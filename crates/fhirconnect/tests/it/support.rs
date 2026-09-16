// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the suites share: the synthetic template of the testkit, the loader
//! for the mapping fixtures beside them, and the diagnostic renderer.

use core::error::Error;
use std::sync::Arc;

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::model::ast::ContextMappingFile;
use fhirconnect::model::ast::ModelMappingFile;
use fhirconnect::model::load::MappingSet;
use fhirconnect::model::parse::lower_context;
use fhirconnect::model::parse::lower_model;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::Program;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::TemplateSource;

/// The fixtures this crate's tests ship.
pub(crate) const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// Builds the index over the testkit's diagnosis template.
pub(crate) fn template() -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::DIAGNOSE_OPT)?;
    Ok(WebTemplateIndex::build(&TemplateSource::Opt14(Box::new(
        opt,
    )))?)
}

/// Compiles the context and model fixture pair named `fixture`.
pub(crate) fn compiled(fixture: &str) -> Result<Arc<Program>, Box<dyn Error>> {
    let mut set = MappingSet::new();
    for (name, text) in [
        (
            format!("contexts/{fixture}.context.yml"),
            std::fs::read_to_string(format!("{FIXTURES}/contexts/{fixture}.context.yml"))?,
        ),
        (
            format!("model/{fixture}.yml"),
            std::fs::read_to_string(format!("{FIXTURES}/model/{fixture}.yml"))?,
        ),
    ] {
        let name = name.as_str();
        let document = openehr_mapping_core::loader::load_str(name, &text)?;
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
    let index = template()?;
    let context = MappingName::new(format!("{fixture}.context"))?;
    compile(
        &set,
        &context,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .map_err(|diagnostics| Box::<dyn Error>::from(render(&diagnostics)))
}

/// Renders a diagnostic list as one error message.
pub(crate) fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{} {}: {}",
                diagnostic.file().display(),
                diagnostic.code(),
                diagnostic.message()
            )
        })
        .collect::<Vec<String>>()
        .join("; ")
}
