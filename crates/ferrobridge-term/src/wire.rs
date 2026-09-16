// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The per-release reader and writer of the FHIR bodies.
//!
//! Each release has its own generated model, so the same code is generated
//! once per release by `per_release` and the functions below dispatch on
//! [`WireVersion`]. The request and response contracts come from
//! `fhir_types::r4::operations` and `fhir_types::r4b::operations`; this module
//! writes no FHIR by hand.
//!
//! NOTE: R4 and R4B declare identical `in` and `out` parameter sets for the
//! three operations (the vendored `OperationDefinition-CodeSystem-lookup`,
//! `-ConceptMap-translate` and `-ValueSet-validate-code` of both packages), so
//! only the decoder differs.

use crate::config::WireVersion;
use crate::error::{BodyError, Outcome};
use crate::outcome::{LookupOutcome, Request, TranslateOutcome, ValidateOutcome};
use http::StatusCode;

/// The `HTTPVerb` code a batch entry carries for the `POST` form of an
/// operation (<https://hl7.org/fhir/R4/valueset-http-verb.html>).
const POST: &str = "POST";

/// The `BundleType` code of a request bundle whose entries are independent
/// (<https://hl7.org/fhir/R4/valueset-bundle-type.html>).
const BATCH: &str = "batch";

/// The `BundleType` code the server answers a `batch` with.
const BATCH_RESPONSE: &str = "batch-response";

/// One entry of a `batch-response` `Bundle`, in request order.
#[derive(Debug)]
pub(crate) struct BatchEntry {
    /// The status `entry.response.status` names.
    pub(crate) status: StatusCode,
    /// `entry.resource`, written back as FHIR JSON.
    pub(crate) body: String,
}

/// Generates the reader and writer of one FHIR release.
macro_rules! per_release {
    ($module:ident, $release:ident) => {
        /// The bodies of one FHIR release, over its own generated model.
        pub(crate) mod $module {
            use super::{BatchEntry, BATCH, BATCH_RESPONSE, POST};
            use crate::concept::{
                Concept, Designation, Equivalence, Match, Property, PropertyValue,
            };
            use crate::error::{BodyError, Issue, Outcome, TxIssue};
            use crate::outcome::{LookupOutcome, Request, TranslateOutcome, ValidateOutcome};
            use fhir_types::$release::bundle::{Bundle, BundleEntry, BundleEntryRequest};
            use fhir_types::$release::coding::Coding;
            use fhir_types::$release::operation_outcome::OperationOutcome;
            use fhir_types::$release::operations::code_system_lookup::CodeSystemLookupRequest;
            use fhir_types::$release::operations::code_system_lookup::CodeSystemLookupResponse;
            use fhir_types::$release::operations::concept_map_translate::ConceptMapTranslateRequest;
            use fhir_types::$release::operations::concept_map_translate::ConceptMapTranslateResponse;
            use fhir_types::$release::operations::value_set_validate_code::ValueSetValidateCodeRequest;
            use fhir_types::$release::operations::value_set_validate_code::ValueSetValidateCodeResponse;
            use fhir_types::$release::parameters::{
                Parameters, ParametersParameter, ParametersParameterValue,
            };
            use fhir_types::$release::resource::Resource;
            use fhir_types::codec::Json;

            /// Returns the `Parameters` that carries `request`.
            fn parameters(request: &Request) -> Parameters {
                match request {
                    Request::Lookup {
                        system,
                        code,
                        version,
                    } => CodeSystemLookupRequest {
                        system: Some(system.as_str().into()),
                        code: Some(code.as_str().into()),
                        version: version.as_deref().map(Into::into),
                        ..Default::default()
                    }
                    .into_parameters(),
                    Request::Translate {
                        system,
                        code,
                        target_system,
                        concept_map_url,
                    } => ConceptMapTranslateRequest {
                        url: concept_map_url.as_deref().map(Into::into),
                        system: Some(system.as_str().into()),
                        code: Some(code.as_str().into()),
                        targetsystem: Some(target_system.as_str().into()),
                        ..Default::default()
                    }
                    .into_parameters(),
                    Request::ValidateCode {
                        value_set_url,
                        system,
                        code,
                    } => ValueSetValidateCodeRequest {
                        url: Some(value_set_url.as_str().into()),
                        system: Some(system.as_str().into()),
                        code: Some(code.as_str().into()),
                        ..Default::default()
                    }
                    .into_parameters(),
                }
            }

            /// Returns the FHIR JSON body that carries `request`.
            pub(crate) fn request_body(request: &Request) -> Result<String, serde_json::Error> {
                serde_json::to_string(&parameters(request))
            }

            /// Returns the FHIR JSON `batch` Bundle that carries `requests`.
            pub(crate) fn batch_body(requests: &[Request]) -> Result<String, serde_json::Error> {
                let bundle = Bundle {
                    r#type: BATCH.into(),
                    entry: requests
                        .iter()
                        .map(|request| BundleEntry {
                            request: Some(BundleEntryRequest {
                                method: POST.into(),
                                url: super::path_of(request).into(),
                                ..Default::default()
                            }),
                            resource: Some(Resource::Parameters(Box::new(parameters(request)))),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                };
                serde_json::to_string(&bundle)
            }

            /// Returns the entries of the `batch-response` `body` carries.
            pub(crate) fn batch_entries(body: &str) -> Result<Vec<BatchEntry>, BodyError> {
                let bundle = serde_json::from_str::<Bundle>(body)?;
                let kind = bundle.r#type.value.clone().unwrap_or_default();
                if kind != BATCH_RESPONSE {
                    return Err(BodyError::BatchType { found: kind });
                }
                bundle.entry.into_iter().map(entry).collect()
            }

            /// Returns one `batch-response` entry as its status and body.
            fn entry(entry: BundleEntry) -> Result<BatchEntry, BodyError> {
                let status = entry
                    .response
                    .as_ref()
                    .and_then(|response| response.status.value.as_deref())
                    .and_then(super::status_of)
                    .ok_or(BodyError::EntryStatus)?;
                let resource = entry.resource.ok_or(BodyError::EntryResource)?;
                Ok(BatchEntry {
                    status,
                    body: serde_json::to_string(&resource)?,
                })
            }

            /// Returns the `OperationOutcome` that `body` carries.
            pub(crate) fn outcome(body: &str) -> Result<Outcome, BodyError> {
                let decoded = serde_json::from_str::<OperationOutcome>(body)?;
                let issues = decoded
                    .issue
                    .iter()
                    .map(|issue| {
                        let details = issue.details.as_ref();
                        Ok(Issue {
                            severity: super::required(
                                issue.severity.value.clone(),
                                "issue.severity",
                            )?,
                            code: super::required(issue.code.value.clone(), "issue.code")?,
                            details: details
                                .and_then(|held| held.text.as_ref())
                                .and_then(|text| text.value.clone()),
                            diagnostics: issue
                                .diagnostics
                                .as_ref()
                                .and_then(|text| text.value.clone()),
                            tx_issue_type: details
                                .and_then(|held| held.coding.iter().find_map(tx_issue)),
                        })
                    })
                    .collect::<Result<Vec<_>, BodyError>>()?;
                Ok(Outcome { issues })
            }

            /// Returns the `tx-issue-type` code `coding` carries, if it is one.
            fn tx_issue(coding: &Coding) -> Option<TxIssue> {
                let system = coding.system.as_ref().and_then(|uri| uri.value.as_deref());
                if system != Some(crate::error::TX_ISSUE_TYPE) {
                    return None;
                }
                Some(TxIssue {
                    code: coding.code.as_ref().and_then(|code| code.value.clone())?,
                    display: coding
                        .display
                        .as_ref()
                        .and_then(|text| text.value.clone()),
                })
            }

            /// Returns the `CodeSystem/$lookup` answer `body` carries.
            pub(crate) fn lookup_found(body: &str) -> Result<LookupOutcome, BodyError> {
                let parameters = serde_json::from_str::<Parameters>(body)?;
                let found = CodeSystemLookupResponse::from_parameters(&parameters)?;
                let designations = found
                    .designation
                    .iter()
                    .map(|designation| {
                        Ok(Designation {
                            language: designation
                                .language
                                .as_ref()
                                .and_then(|code| code.value.clone()),
                            usage: designation.r#use.as_ref().map(concept),
                            value: super::required(
                                designation.value.value.clone(),
                                "designation.value",
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, BodyError>>()?;
                let properties = found
                    .property
                    .iter()
                    .map(|property| {
                        Ok(Property {
                            code: super::required(
                                property.code.value.clone(),
                                "property.code",
                            )?,
                            value: property.value.clone().map(property_value).transpose()?,
                            description: property
                                .description
                                .as_ref()
                                .and_then(|text| text.value.clone()),
                        })
                    })
                    .collect::<Result<Vec<_>, BodyError>>()?;
                Ok(LookupOutcome::Found {
                    name: super::required(found.name.value.clone(), "name")?,
                    version: found.version.as_ref().and_then(|text| text.value.clone()),
                    display: super::required(found.display.value.clone(), "display")?,
                    designations,
                    properties,
                })
            }

            /// Returns the `ConceptMap/$translate` answer `body` carries.
            pub(crate) fn translate(body: &str) -> Result<TranslateOutcome, BodyError> {
                let parameters = serde_json::from_str::<Parameters>(body)?;
                let answer = ConceptMapTranslateResponse::from_parameters(&parameters)?;
                let message = answer.message.as_ref().and_then(|text| text.value.clone());
                if !super::required_flag(answer.result.value, "result")? {
                    return Ok(TranslateOutcome::NoMatch { message });
                }
                let matches = answer
                    .r#match
                    .iter()
                    .map(|found| Match {
                        equivalence: found
                            .equivalence
                            .as_ref()
                            .and_then(|code| code.value.as_deref())
                            .map(Equivalence::new),
                        concept: found.concept.as_ref().map(concept),
                        source: found.source.as_ref().and_then(|url| url.value.clone()),
                    })
                    .collect();
                Ok(TranslateOutcome::Translated { matches, message })
            }

            /// Returns the `ValueSet/$validate-code` answer `body` carries.
            pub(crate) fn validate_code(body: &str) -> Result<ValidateOutcome, BodyError> {
                let parameters = serde_json::from_str::<Parameters>(body)?;
                let answer = ValueSetValidateCodeResponse::from_parameters(&parameters)?;
                let message = answer.message.as_ref().and_then(|text| text.value.clone());
                if super::required_flag(answer.result.value, "result")? {
                    return Ok(ValidateOutcome::Valid {
                        display: answer.display.as_ref().and_then(|text| text.value.clone()),
                        message,
                    });
                }
                let issues = answer
                    .issues
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?
                    .map(|text| outcome(&text))
                    .transpose()?;
                Ok(ValidateOutcome::Invalid {
                    message,
                    outcome: issues,
                })
            }

            /// Returns `coding` in the release-neutral shape.
            fn concept(coding: &Coding) -> Concept {
                Concept {
                    system: coding.system.as_ref().and_then(|uri| uri.value.clone()),
                    version: coding.version.as_ref().and_then(|text| text.value.clone()),
                    code: coding.code.as_ref().and_then(|code| code.value.clone()),
                    display: coding.display.as_ref().and_then(|text| text.value.clone()),
                }
            }

            /// Returns one property value as the choice member that carried it
            /// and that member's lexical JSON.
            fn property_value(value: ParametersParameterValue) -> Result<PropertyValue, BodyError> {
                let parameter = ParametersParameter {
                    name: "value".into(),
                    value: Some(value),
                    ..Default::default()
                };
                parameter
                    .to_json()?
                    .into_iter()
                    .find(|(key, _)| key.starts_with("value"))
                    .map(|(member, value)| PropertyValue { member, value })
                    .ok_or(BodyError::MissingValue {
                        parameter: "property.value",
                    })
            }
        }
    };
}

per_release!(r4, r4);
per_release!(r4b, r4b);

/// Returns the relative URL of `request`, the form a batch entry carries
/// (<https://hl7.org/fhir/R4/http.html#transaction>).
fn path_of(request: &Request) -> &'static str {
    match request {
        Request::Lookup { .. } => crate::LOOKUP,
        Request::Translate { .. } => crate::TRANSLATE,
        Request::ValidateCode { .. } => crate::VALIDATE_CODE,
    }
}

/// Returns the status an HTTP status line names, `None` when it names none.
///
/// `Bundle.entry.response.status` is "the status code returned by processing
/// this entry", written as the status line's code and optional reason phrase
/// (<https://hl7.org/fhir/R4/bundle-definitions.html#Bundle.entry.response.status>).
fn status_of(line: &str) -> Option<StatusCode> {
    let code = line.split_whitespace().next()?;
    StatusCode::from_bytes(code.as_bytes()).ok()
}

/// Returns the value of a primitive the operation declares as mandatory.
///
/// A FHIR primitive can be present with an extension and no value at all
/// (<https://hl7.org/fhir/R4/json.html#primitive>), so an answer whose
/// `display` states nothing is a defective answer, never an empty string.
fn required(value: Option<String>, parameter: &'static str) -> Result<String, BodyError> {
    value.ok_or(BodyError::MissingValue { parameter })
}

/// Returns the value of a mandatory `boolean` out parameter.
fn required_flag(value: Option<bool>, parameter: &'static str) -> Result<bool, BodyError> {
    value.ok_or(BodyError::MissingValue { parameter })
}

/// Returns the `OperationOutcome` that `body` carries, read in `version`.
pub(crate) fn outcome(version: WireVersion, body: &str) -> Result<Outcome, BodyError> {
    match version {
        WireVersion::R4 => r4::outcome(body),
        WireVersion::R4B => r4b::outcome(body),
    }
}

/// Returns the FHIR JSON body that carries `request`, written in `version`.
pub(crate) fn request_body(
    version: WireVersion,
    request: &Request,
) -> Result<String, serde_json::Error> {
    match version {
        WireVersion::R4 => r4::request_body(request),
        WireVersion::R4B => r4b::request_body(request),
    }
}

/// Returns the FHIR JSON `batch` Bundle that carries `requests`.
pub(crate) fn batch_body(
    version: WireVersion,
    requests: &[Request],
) -> Result<String, serde_json::Error> {
    match version {
        WireVersion::R4 => r4::batch_body(requests),
        WireVersion::R4B => r4b::batch_body(requests),
    }
}

/// Returns the entries of the `batch-response` `body` carries.
pub(crate) fn batch_entries(
    version: WireVersion,
    body: &str,
) -> Result<Vec<BatchEntry>, BodyError> {
    match version {
        WireVersion::R4 => r4::batch_entries(body),
        WireVersion::R4B => r4b::batch_entries(body),
    }
}

/// Returns the `CodeSystem/$lookup` answer `body` carries.
pub(crate) fn lookup_found(version: WireVersion, body: &str) -> Result<LookupOutcome, BodyError> {
    match version {
        WireVersion::R4 => r4::lookup_found(body),
        WireVersion::R4B => r4b::lookup_found(body),
    }
}

/// Returns the `ConceptMap/$translate` answer `body` carries.
pub(crate) fn translate(version: WireVersion, body: &str) -> Result<TranslateOutcome, BodyError> {
    match version {
        WireVersion::R4 => r4::translate(body),
        WireVersion::R4B => r4b::translate(body),
    }
}

/// Returns the `ValueSet/$validate-code` answer `body` carries.
pub(crate) fn validate_code(
    version: WireVersion,
    body: &str,
) -> Result<ValidateOutcome, BodyError> {
    match version {
        WireVersion::R4 => r4::validate_code(body),
        WireVersion::R4B => r4b::validate_code(body),
    }
}

#[cfg(test)]
mod tests {
    use super::{batch_body, request_body, status_of};
    use crate::config::WireVersion;
    use crate::outcome::Request;
    use http::StatusCode;

    fn lookup() -> Request {
        Request::Lookup {
            system: "http://example.org/cs".to_owned(),
            code: "a".to_owned(),
            version: Some("1.0.0".to_owned()),
        }
    }

    #[test]
    fn a_status_line_reads_its_code() {
        assert_eq!(Some(StatusCode::OK), status_of("200 OK"));
        assert_eq!(Some(StatusCode::BAD_REQUEST), status_of("400"));
        assert_eq!(None, status_of(""));
        assert_eq!(None, status_of("not a status"));
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn both_releases_write_the_same_lookup_parameters() -> Result<(), serde_json::Error> {
        let r4 = request_body(WireVersion::R4, &lookup())?;
        let r4b = request_body(WireVersion::R4B, &lookup())?;
        assert_eq!(r4, r4b);
        assert!(r4.contains(r#""name":"system""#), "{r4}");
        assert!(r4.contains(r#""name":"code""#), "{r4}");
        assert!(r4.contains(r#""name":"version""#), "{r4}");
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn a_batch_bundle_carries_one_post_entry_per_request() -> Result<(), serde_json::Error> {
        let body = batch_body(WireVersion::R4, &[lookup(), lookup()])?;
        assert!(body.contains(r#""type":"batch""#), "{body}");
        assert_eq!(2, body.matches(r#""method":"POST""#).count(), "{body}");
        assert_eq!(
            2,
            body.matches(r#""url":"CodeSystem/$lookup""#).count(),
            "{body}"
        );
        Ok(())
    }
}
