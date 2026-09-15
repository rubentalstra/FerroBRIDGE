// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Turning a stored composition into the resource the facade answers with.
//!
//! The engine maps the composition back to FHIR, and this module adds the
//! three facts the engine has no way to know: the logical id the identity map
//! holds, the `meta.versionId` the CDR's version tree names, and the
//! `meta.source` that points at the composition version the content came from
//! (`docs/architecture.md` §9).

use ferrobridge_openehr::ids::EhrId;
use ferrobridge_openehr::ids::ObjectVersionId;
use fhir_types::codec::Object;
use fhir_types::codec::Value;
use fhirconnect::engine::outcome::Warning;
use fhirconnect::resolve::program::Program;
use http::StatusCode;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;
use url::Url;

use crate::facade::engine;
use crate::facade::handlers::Refusal;
use crate::facade::identity::FhirResourceId;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::reply;

/// What one rendered resource carries beside its body.
#[derive(Debug, Clone)]
pub(crate) struct Rendered {
    /// The resource, ready for the wire.
    pub(crate) body: Object,
    /// The losses the run declared, as issues a caller may report.
    pub(crate) warnings: Vec<Warning>,
}

/// Returns the resource `composition` maps to, decorated with its identity.
pub(crate) fn render(
    program: &Program,
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    id: &FhirResourceId,
    version: &ObjectVersionId,
    source: &Url,
) -> Result<Rendered, Refusal> {
    let produced =
        engine::outbound(program, index, composition).map_err(|error| engine_refusal(&error))?;
    let warnings = produced.warnings().to_vec();
    let mut body = match produced.into_value() {
        Value::Object(object) => object,
        other => {
            return Err(reply::refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                Issue::error(IssueType::Exception).diagnosing(format!(
                    "the engine produced {other:?} where a FHIR resource object belongs"
                )),
            ));
        }
    };
    body.insert(String::from("id"), Value::String(String::from(id.as_str())));
    let mut meta = body
        .get("meta")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    meta.insert(
        String::from("versionId"),
        Value::String(String::from(version.version_tree_id())),
    );
    // NOTE: `Meta.source` is "a uri that identifies the source system"
    // (<https://hl7.org/fhir/R4/resource.html#Meta>), so the openEHR version
    // uid travels as the ITS-REST URL of that exact version.
    meta.insert(String::from("source"), Value::String(source.to_string()));
    meta.insert(
        String::from("profile"),
        Value::Array(vec![Value::String(String::from(
            program.profile().url().as_str(),
        ))]),
    );
    body.insert(String::from("meta"), Value::Object(meta));
    Ok(Rendered { body, warnings })
}

/// Records the declared set of losses one outbound run took.
///
/// A warning is a loss the specification itself declares, never a swallowed
/// failure, and it carries a mapping name and an element path rather than a
/// value, so the line holds no clinical content.
pub(crate) fn log_warnings(rendered: &Rendered) {
    if rendered.warnings.is_empty() {
        return;
    }
    tracing::debug!(
        declared = rendered.warnings.len(),
        losses = rendered
            .warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>()
            .join("; "),
        "the outbound mapping declared losses"
    );
}

/// Returns the ITS-REST URL of one composition version.
pub(crate) fn composition_url(
    base: &Url,
    ehr_id: &EhrId,
    version: &ObjectVersionId,
) -> Result<Url, Refusal> {
    base.join(&format!("ehr/{}/composition/{version}", ehr_id.as_str()))
        .map_err(|error| {
            reply::refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                Issue::error(IssueType::Exception).diagnosing(format!(
                    "the composition URL could not be built from the configured CDR base: {error}"
                )),
            )
        })
}

/// Returns the refusal one engine error renders as.
///
/// "An element the program cannot map refuses the unit"
/// (`docs/architecture.md` §9), so the answer is a `422` whose diagnostics
/// name the mapping and the element the engine refused at.
pub(crate) fn engine_refusal(error: &fhirconnect::engine::traverse::EngineError) -> Refusal {
    reply::refusal(
        StatusCode::UNPROCESSABLE_ENTITY,
        Issue::error(IssueType::Processing)
            .diagnosing(chain(error))
            .detailing("the mapping refused this resource"),
    )
}

/// Returns `error` and every cause behind it as one line.
pub(crate) fn chain(error: &dyn core::error::Error) -> String {
    let mut line = error.to_string();
    let mut cause = error.source();
    while let Some(source) = cause {
        line.push_str(": ");
        line.push_str(&source.to_string());
        cause = source.source();
    }
    line
}

#[cfg(test)]
mod tests {
    use super::{chain, composition_url};
    use ferrobridge_openehr::ids::{EhrId, ObjectVersionId};

    #[test]
    fn the_source_url_names_the_ehr_and_the_exact_version() {
        let base: url::Url = "http://cdr.invalid/openehr/v1/"
            .parse()
            .expect("a legal base URL");
        let ehr = EhrId::new("bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11").expect("a legal ehr_id");
        let version = ObjectVersionId::new("8849182c-82ad-4088-a07f-48ead4180515::ferroehr::2")
            .expect("a legal version id");
        let built = composition_url(&base, &ehr, &version).expect("the URL builds");
        assert_eq!(
            "http://cdr.invalid/openehr/v1/ehr/bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11/composition/8849182c-82ad-4088-a07f-48ead4180515::ferroehr::2",
            built.as_str()
        );
    }

    #[test]
    fn a_cause_chain_renders_as_one_line() {
        let error = crate::facade::identity::IdError::Empty { kind: "FHIR id" };
        assert_eq!("a FHIR id cannot be empty", chain(&error));
    }
}
