// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two contracts against the operation definitions the vendored FSH
//! declares.

use core::error::Error;

use ferrobridge_testkit::conformance::Case;
use ferrobridge_testkit::conformance::Corpus;
use ferrobridge_testkit::conformance::record;
use fhir_types::r4::parameters::Parameters;
use fhir_types::r4::parameters::ParametersParameter;
use fhir_types::r4::parameters::ParametersParameterValue;
use fhir_types::r4::primitives::Code;
use fhir_types::r4::reference::Reference;
use fhir_types::r4::resource::Resource;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::traverse::functions::NoMappingFunctions;
use fhirconnect::operations::contract::Format;
use fhirconnect::operations::contract::ToFhirRequest;
use fhirconnect::operations::contract::ToOpenehrRequest;
use fhirconnect::operations::run;

use crate::operations::SUBJECT_CONTEXT;
use crate::operations::SUBJECT_PROFILE;
use crate::operations::TEMPLATE;
use crate::operations::bundle;
use crate::operations::committed_composition;
use crate::operations::condition;
use crate::operations::program_set;
use crate::operations::reference;
use crate::operations::settings;

/// The EHR the draft chapter's own examples name.
const EHR_ID: &str = "53d89df2-5501-4455-9a65-565a5d1ddb7c";

/// The vendored FSH operation definitions of the draft chapter.
const DRAFT_FSH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/fhirconnect/draft-rest-api/rest/input/fsh/operations"
);

/// One parameter an FSH `OperationDefinition` declares.
#[derive(Debug, Default)]
struct Declared {
    /// The parameter name, without the `#` the FSH code syntax carries.
    name: String,
    /// `in` or `out`.
    usage: String,
    /// The `min` cardinality, as the FSH writes it.
    min: String,
    /// The type code, without the `#`.
    kind: String,
    /// The names of the parts the parameter groups.
    parts: Vec<String>,
}

/// Reads the parameters one vendored FSH operation definition declares.
///
/// FSH assigns one path per line, so a parameter block opens with
/// `* parameter[+]` and its members are indented under it
/// (<https://hl7.org/fhir/uv/shorthand/reference.html>).
fn fsh(file: &str) -> Result<Vec<Declared>, Box<dyn Error>> {
    let text = std::fs::read_to_string(format!("{DRAFT_FSH}/{file}"))?;
    let mut declared: Vec<Declared> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line == "* parameter[+]" {
            declared.push(Declared::default());
            continue;
        }
        let Some(last) = declared.last_mut() else {
            continue;
        };
        if let Some(name) = line.strip_prefix("  * name = #") {
            last.name = String::from(name);
        } else if let Some(usage) = line.strip_prefix("  * use = #") {
            last.usage = String::from(usage);
        } else if let Some(min) = line.strip_prefix("  * min = ") {
            last.min = String::from(min);
        } else if let Some(kind) = line.strip_prefix("  * type = #") {
            last.kind = String::from(kind);
        } else if let Some(part) = line.strip_prefix("    * name = #") {
            last.parts.push(String::from(part));
        }
    }
    Ok(declared)
}

/// Returns a parameter carrying `text` as a `valueString`.
fn string_parameter(name: &str, text: &str) -> ParametersParameter {
    ParametersParameter {
        name: fhir_types::r4::primitives::String::from(name),
        value: Some(ParametersParameterValue::String(
            fhir_types::r4::primitives::String::from(text),
        )),
        ..ParametersParameter::default()
    }
}

/// Returns a parameter carrying a literal reference as a `valueReference`.
fn reference_parameter(name: &str, text: &str) -> ParametersParameter {
    ParametersParameter {
        name: fhir_types::r4::primitives::String::from(name),
        value: Some(ParametersParameterValue::Reference(Box::new(reference(
            text,
        )))),
        ..ParametersParameter::default()
    }
}

/// Returns the literal reference `carried` holds.
fn literal(carried: Option<&Reference>) -> Option<String> {
    carried
        .and_then(|value| value.reference.as_ref())
        .and_then(|value| value.value.clone())
}

/// Builds the `$tofhir` input the declared `in` parameters of `ToFhir.fsh`
/// name, one value per parameter and per context part.
fn tofhir_parameters(declared: &[Declared]) -> Result<Parameters, Box<dyn Error>> {
    let committed = committed_composition()?;
    let mut carried = Parameters::default();
    for input in declared.iter().filter(|parameter| parameter.usage == "in") {
        match input.name.as_str() {
            "composition" => carried
                .parameter
                .push(string_parameter(&input.name, &committed.to_string())),
            "templateId" => carried
                .parameter
                .push(string_parameter(&input.name, TEMPLATE)),
            "context" => {
                let mut group = ParametersParameter {
                    name: fhir_types::r4::primitives::String::from(input.name.as_str()),
                    ..ParametersParameter::default()
                };
                for part in &input.parts {
                    group.part.push(match part.as_str() {
                        "ehr_id" => string_parameter(part, EHR_ID),
                        "patient" => reference_parameter(part, "Patient/123"),
                        "who" => reference_parameter(part, "Practitioner/456"),
                        "onBehalfOf" => reference_parameter(part, "Organization/charite"),
                        other => {
                            return Err(Box::<dyn Error>::from(format!(
                                "ToFhir.fsh declares the context part `{other}`, which the contract does not read"
                            )));
                        }
                    });
                }
                carried.parameter.push(group);
            }
            other => {
                return Err(Box::<dyn Error>::from(format!(
                    "ToFhir.fsh declares `{other}`, which the contract does not read"
                )));
            }
        }
    }
    Ok(carried)
}

/// Builds the `$toopenehr` input the declared `in` parameters of
/// `ToOpenEhr.fsh` name, one value per parameter.
fn toopenehr_parameters(declared: &[Declared]) -> Result<Parameters, Box<dyn Error>> {
    let mut carried = Parameters::default();
    for input in declared.iter().filter(|parameter| parameter.usage == "in") {
        match input.name.as_str() {
            "bundle" => carried.parameter.push(ParametersParameter {
                name: fhir_types::r4::primitives::String::from(input.name.as_str()),
                resource: Some(Resource::Bundle(Box::new(bundle(vec![condition(
                    SUBJECT_PROFILE,
                )?])?))),
                ..ParametersParameter::default()
            }),
            "templateId" => carried
                .parameter
                .push(string_parameter(&input.name, TEMPLATE)),
            "format" => carried.parameter.push(ParametersParameter {
                name: fhir_types::r4::primitives::String::from(input.name.as_str()),
                value: Some(ParametersParameterValue::Code(Code::from("flat"))),
                ..ParametersParameter::default()
            }),
            other => {
                return Err(Box::<dyn Error>::from(format!(
                    "ToOpenEhr.fsh declares `{other}`, which the contract does not read"
                )));
            }
        }
    }
    Ok(carried)
}

#[test]
fn the_tofhir_contract_reads_the_parameter_set_the_vendored_fsh_declares()
-> Result<(), Box<dyn Error>> {
    // ToFhir.fsh (draft) at the pinned commit, its declared parameter set.
    let declared = fsh("ToFhir.fsh")?;
    let outputs: Vec<&str> = declared
        .iter()
        .filter(|parameter| parameter.usage == "out")
        .map(|parameter| parameter.name.as_str())
        .collect();
    assert_eq!(outputs, vec!["return"], "$tofhir answers one out parameter");

    let request = ToFhirRequest::from_parameters(&tofhir_parameters(&declared)?)?;
    let context = request.context().ok_or("the context group was read")?;
    assert_eq!(context.ehr_id(), Some(EHR_ID));
    assert_eq!(
        literal(context.patient()),
        Some(String::from("Patient/123"))
    );
    assert_eq!(
        literal(context.who()),
        Some(String::from("Practitioner/456"))
    );
    assert_eq!(
        literal(context.on_behalf_of()),
        Some(String::from("Organization/charite"))
    );
    Ok(())
}

#[test]
fn the_toopenehr_contract_reads_the_parameter_set_the_vendored_fsh_declares()
-> Result<(), Box<dyn Error>> {
    // ToOpenEhr.fsh (draft) at the pinned commit, its declared parameter set.
    let declared = fsh("ToOpenEhr.fsh")?;
    let outputs: Vec<&str> = declared
        .iter()
        .filter(|parameter| parameter.usage == "out")
        .map(|parameter| parameter.name.as_str())
        .collect();
    assert_eq!(
        outputs,
        vec!["composition", "outcome"],
        "$toopenehr answers the composition and an optional outcome"
    );

    let request = ToOpenehrRequest::from_parameters(&toopenehr_parameters(&declared)?)?;
    assert_eq!(request.format(), Format::Flat);
    assert_eq!(
        request.template_id().map(|id| String::from(id.as_str())),
        Some(String::from(TEMPLATE))
    );
    Ok(())
}

/// Returns why an answer's parameters depart from the declared `out` set: a
/// name the definition does not declare, or a `min = 1` parameter missing.
fn undeclared_out(declared: &[Declared], answered: &Parameters) -> Option<String> {
    let outs: Vec<&Declared> = declared
        .iter()
        .filter(|parameter| parameter.usage == "out")
        .collect();
    let names: Vec<&str> = answered
        .parameter
        .iter()
        .filter_map(|parameter| parameter.name.value.as_deref())
        .collect();
    if let Some(name) = names
        .iter()
        .find(|name| !outs.iter().any(|out| out.name == **name))
    {
        return Some(format!(
            "the answer carries `{name}`, which is not declared"
        ));
    }
    outs.iter()
        .find(|out| out.min == "1" && !names.contains(&out.name.as_str()))
        .map(|out| format!("the answer omits `{}`, which is declared min 1", out.name))
}

/// Returns the verdict on `ToFhir.fsh`: its inputs are read, and the run
/// answers the single `return` Bundle it declares.
fn tofhir_verdict() -> Result<Option<String>, Box<dyn Error>> {
    let declared = fsh("ToFhir.fsh")?;
    let outs: Vec<(&str, &str)> = declared
        .iter()
        .filter(|parameter| parameter.usage == "out")
        .map(|parameter| (parameter.name.as_str(), parameter.kind.as_str()))
        .collect();
    // NOTE: rest-api.adoc (draft) $tofhir Output: "The response MUST be a FHIR `Bundle`
    // containing the mapped resources", so the one declared `return` is that Bundle.
    if outs != [("return", "Bundle")] {
        return Ok(Some(format!(
            "ToFhir.fsh declares {outs:?}, which the operation does not answer"
        )));
    }
    let request = match ToFhirRequest::from_parameters(&tofhir_parameters(&declared)?) {
        Ok(request) => request,
        Err(error) => return Ok(Some(format!("the contract refuses the inputs: {error}"))),
    };
    let set = program_set(&[SUBJECT_CONTEXT])?;
    Ok(
        match run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request) {
            Ok(answer) if answer.bundle().entry.is_empty() => {
                Some(String::from("the answered Bundle is empty"))
            }
            Ok(_) => None,
            Err(error) => Some(format!(
                "the operation refuses the declared inputs: {error}"
            )),
        },
    )
}

/// Returns the verdict on `ToOpenEhr.fsh`: its inputs are read, and the run
/// answers only declared `out` parameters, every `min = 1` one present.
fn toopenehr_verdict() -> Result<Option<String>, Box<dyn Error>> {
    let declared = fsh("ToOpenEhr.fsh")?;
    let request = match ToOpenehrRequest::from_parameters(&toopenehr_parameters(&declared)?) {
        Ok(request) => request,
        Err(error) => return Ok(Some(format!("the contract refuses the inputs: {error}"))),
    };
    let set = program_set(&[SUBJECT_CONTEXT])?;
    Ok(
        match run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request) {
            Ok(answer) => undeclared_out(&declared, &answer.to_parameters()),
            Err(error) => Some(format!(
                "the operation refuses the declared inputs: {error}"
            )),
        },
    )
}

/// The conformance verdict on every FSH operation definition of the draft
/// REST API chapter: the wire contract reads every declared input, and the
/// operation answers as the definition declares (rest-api.adoc, draft).
#[test]
fn conformance_the_draft_operation_definitions_hold_their_pass_list() -> Result<(), Box<dyn Error>>
{
    let verdict = |id: &str, failure: Option<String>| match failure {
        Some(reason) => Case::fail(id, reason),
        None => Case::pass(id),
    };
    let cases = [
        verdict("ToFhir.fsh", tofhir_verdict()?),
        verdict("ToOpenEhr.fsh", toopenehr_verdict()?),
    ];
    let outcome = record(Corpus::DraftRestApi, &cases)?;
    assert!(
        outcome.regressed.is_empty(),
        "cases the pass list records no longer pass: {:?}",
        outcome.regressed
    );
    Ok(())
}
