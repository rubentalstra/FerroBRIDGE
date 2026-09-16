// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `wiremock` responses in the documented upstream shapes.
//!
//! [`its_rest`] carries the openEHR answers ITS-REST 1.1.0 documents and
//! [`terminology`] the FHIR R4 `Parameters` the three terminology operations
//! return, so a client test states which documented answer it stubs instead of
//! rebuilding the body.

/// The openEHR ITS-REST 1.1.0 answers, by status.
pub mod its_rest {
    use wiremock::ResponseTemplate;

    /// The canonical JSON media type of ITS-REST 1.1.0.
    const CANONICAL_JSON: &str = "application/json";

    /// Returns the `201` of a resource creation.
    ///
    /// "The `ETag` (i.e. entity tag) response header is the `version_uid`
    /// identifier, enclosed by double quotes"
    /// (`ehr-codegen.openapi.yaml`, `components.headers.ETag_Version`), and
    /// `Location` is the absolute URL of the new resource.
    #[must_use]
    pub fn created(entity_tag: &str, location: &str, body: &str) -> ResponseTemplate {
        ResponseTemplate::new(201)
            .insert_header("ETag", format!("W/\"{entity_tag}\""))
            .insert_header("Location", location)
            .insert_header("Content-Type", CANONICAL_JSON)
            .set_body_string(body.to_owned())
    }

    /// Returns the `422` of content that is well formed and fails semantic
    /// validation, with the `Error` body of the `OpenAPI` schema.
    ///
    /// The schema requires `message` and `validationErrors`
    /// (`ehr-codegen.openapi.yaml`, `components.schemas.Error`).
    #[must_use]
    pub fn unprocessable(message: &str, validation_errors: &[&str]) -> ResponseTemplate {
        ResponseTemplate::new(422)
            .insert_header("Content-Type", CANONICAL_JSON)
            .set_body_string(error_body(message, validation_errors))
    }

    /// Returns the `412` of a `PUT` whose `If-Match` does not name the latest
    /// version, with that latest version in its `ETag`.
    #[must_use]
    pub fn precondition_failed(latest_version_id: &str) -> ResponseTemplate {
        ResponseTemplate::new(412).insert_header("ETag", format!("W/\"{latest_version_id}\""))
    }

    /// Returns the `204` a `DELETE` answers, naming the deleting version in
    /// its `ETag`.
    #[must_use]
    pub fn deleted(version_id: &str) -> ResponseTemplate {
        ResponseTemplate::new(204).insert_header("ETag", format!("W/\"{version_id}\""))
    }

    /// Returns the `204` a `GET` of a deleted composition answers, which
    /// carries no body and no `ETag`.
    #[must_use]
    pub fn no_content() -> ResponseTemplate {
        ResponseTemplate::new(204)
    }

    /// Returns the `404` of a resource that does not exist.
    #[must_use]
    pub fn not_found(message: &str) -> ResponseTemplate {
        ResponseTemplate::new(404)
            .insert_header("Content-Type", CANONICAL_JSON)
            .set_body_string(error_body(message, &[]))
    }

    /// Returns the `Error` body of the `OpenAPI` schema as canonical JSON.
    fn error_body(message: &str, validation_errors: &[&str]) -> String {
        serde_json::json!({
            "message": message,
            "validationErrors": validation_errors,
        })
        .to_string()
    }
}

/// The FHIR `Parameters`, `OperationOutcome` and `Bundle` the terminology
/// operations return.
///
/// R4 and R4B write these bodies identically, so one set of shapes stubs both
/// releases; only the base path a server serves them under differs.
pub mod terminology {
    use wiremock::ResponseTemplate;

    /// The FHIR JSON media type (<https://hl7.org/fhir/R4/http.html#mime-type>).
    const FHIR_JSON: &str = "application/fhir+json";

    /// The `tx-issue-type` code system of the FHIR tools implementation guide
    /// (<https://build.fhir.org/ig/FHIR/fhir-tools-ig/CodeSystem-tx-issue-type.html>).
    pub const TX_ISSUE_TYPE: &str = "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type";

    /// One `designation` part of a `CodeSystem/$lookup` answer, as a language
    /// and its text value.
    pub type Designation<'a> = (&'a str, &'a str);

    /// One `property` part of a `CodeSystem/$lookup` answer, as a code, the
    /// `value[x]` member name, and the value that member carries.
    pub type Property<'a> = (&'a str, &'a str, serde_json::Value);

    /// Returns the `Parameters` of `CodeSystem/$lookup`.
    ///
    /// `name` and `display` are the two mandatory out parameters
    /// (`OperationDefinition-CodeSystem-lookup`, R4).
    #[must_use]
    pub fn lookup(name: &str, display: &str, version: Option<&str>) -> ResponseTemplate {
        fhir_parameters(&lookup_body(name, display, version, &[], &[]))
    }

    /// Returns the `Parameters` of `CodeSystem/$lookup` with its `designation`
    /// and `property` parts.
    #[must_use]
    pub fn lookup_detailed(
        name: &str,
        display: &str,
        version: Option<&str>,
        designations: &[Designation<'_>],
        properties: &[Property<'_>],
    ) -> ResponseTemplate {
        fhir_parameters(&lookup_body(
            name,
            display,
            version,
            designations,
            properties,
        ))
    }

    /// Returns the out parameters of `CodeSystem/$lookup`.
    #[must_use]
    pub fn lookup_body(
        name: &str,
        display: &str,
        version: Option<&str>,
        designations: &[Designation<'_>],
        properties: &[Property<'_>],
    ) -> Vec<serde_json::Value> {
        let mut parameters = vec![
            serde_json::json!({"name": "name", "valueString": name}),
            serde_json::json!({"name": "display", "valueString": display}),
        ];
        if let Some(version) = version {
            parameters.push(serde_json::json!({"name": "version", "valueString": version}));
        }
        for (language, value) in designations {
            parameters.push(serde_json::json!({
                "name": "designation",
                "part": [
                    {"name": "language", "valueCode": language},
                    {"name": "value", "valueString": value},
                ],
            }));
        }
        for (code, member, value) in properties {
            let mut held = serde_json::Map::new();
            held.insert("name".to_owned(), serde_json::Value::from("value"));
            held.insert((*member).to_owned(), value.clone());
            parameters.push(serde_json::json!({
                "name": "property",
                "part": [{"name": "code", "valueCode": code}, held],
            }));
        }
        parameters
    }

    /// Returns the `Parameters` of `ConceptMap/$translate` with one match.
    ///
    /// `result` is mandatory and each `match` carries `equivalence` and
    /// `concept` (`OperationDefinition-ConceptMap-translate`, R4).
    #[must_use]
    pub fn translate(
        equivalence: &str,
        target_system: &str,
        target_code: &str,
        target_display: &str,
    ) -> ResponseTemplate {
        fhir_parameters(&translate_body(&[(
            equivalence,
            target_system,
            target_code,
            target_display,
        )]))
    }

    /// Returns the out parameters of `ConceptMap/$translate`, one `match` per
    /// entry of `matches`.
    #[must_use]
    pub fn translate_body(matches: &[(&str, &str, &str, &str)]) -> Vec<serde_json::Value> {
        let mut parameters = vec![serde_json::json!({"name": "result", "valueBoolean": true})];
        for (equivalence, system, code, display) in matches {
            parameters.push(serde_json::json!({
                "name": "match",
                "part": [
                    {"name": "equivalence", "valueCode": equivalence},
                    {"name": "concept", "valueCoding": {
                        "system": system,
                        "code": code,
                        "display": display,
                    }},
                ],
            }));
        }
        parameters
    }

    /// Returns the `Parameters` of `ConceptMap/$translate` with several
    /// matches, so a caller can check one equivalence per match.
    #[must_use]
    pub fn translate_matches(matches: &[(&str, &str, &str, &str)]) -> ResponseTemplate {
        fhir_parameters(&translate_body(matches))
    }

    /// Returns the `Parameters` of `ConceptMap/$translate` with no match.
    ///
    /// A `result` of `false` with a `message` is the documented shape of a
    /// translation that found nothing
    /// (`OperationDefinition-ConceptMap-translate`, R4).
    #[must_use]
    pub fn translate_no_match(message: &str) -> ResponseTemplate {
        fhir_parameters(&[
            serde_json::json!({"name": "result", "valueBoolean": false}),
            serde_json::json!({"name": "message", "valueString": message}),
        ])
    }

    /// Returns the `Parameters` of `ValueSet/$validate-code`.
    ///
    /// `result` is mandatory, `message` and `display` are optional
    /// (`OperationDefinition-ValueSet-validate-code`, R4).
    #[must_use]
    pub fn validate_code(
        result: bool,
        display: Option<&str>,
        message: Option<&str>,
    ) -> ResponseTemplate {
        fhir_parameters(&validate_code_body(result, display, message, None))
    }

    /// Returns the out parameters of `ValueSet/$validate-code`.
    ///
    /// `issues` is the itemised `OperationOutcome` the terminology ecosystem
    /// asks for beside a false `result`
    /// (<https://hl7.org/fhir/uv/tx-ecosystem/requirements.html>).
    #[must_use]
    pub fn validate_code_body(
        result: bool,
        display: Option<&str>,
        message: Option<&str>,
        issues: Option<serde_json::Value>,
    ) -> Vec<serde_json::Value> {
        let mut parameters = vec![serde_json::json!({"name": "result", "valueBoolean": result})];
        if let Some(display) = display {
            parameters.push(serde_json::json!({"name": "display", "valueString": display}));
        }
        if let Some(message) = message {
            parameters.push(serde_json::json!({"name": "message", "valueString": message}));
        }
        if let Some(issues) = issues {
            parameters.push(serde_json::json!({"name": "issues", "resource": issues}));
        }
        parameters
    }

    /// Returns the `Parameters` of a `ValueSet/$validate-code` that refused
    /// the code, with the `issues` outcome beside the message.
    #[must_use]
    pub fn validate_code_invalid(message: &str, tx_issue_type: &str) -> ResponseTemplate {
        let issues = outcome_body("code-invalid", Some(tx_issue_type), message);
        fhir_parameters(&validate_code_body(
            false,
            None,
            Some(message),
            Some(issues),
        ))
    }

    /// Returns the `OperationOutcome` body of a refused operation.
    ///
    /// A failed operation answers "an `OperationOutcome` resource with error
    /// details" (<https://hl7.org/fhir/R4/operations.html>), and a terminology
    /// server classifies the failure with a [`TX_ISSUE_TYPE`] coding in
    /// `issue.details.coding`.
    #[must_use]
    pub fn outcome_body(
        issue_code: &str,
        tx_issue_type: Option<&str>,
        diagnostics: &str,
    ) -> serde_json::Value {
        let mut details = serde_json::json!({"text": diagnostics});
        if let Some(tx_issue_type) = tx_issue_type {
            details = serde_json::json!({
                "coding": [{"system": TX_ISSUE_TYPE, "code": tx_issue_type}],
                "text": diagnostics,
            });
        }
        serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": issue_code,
                "details": details,
                "diagnostics": diagnostics,
            }],
        })
    }

    /// Returns `status` carrying the `OperationOutcome` of a refused
    /// operation.
    #[must_use]
    pub fn outcome(
        status: u16,
        issue_code: &str,
        tx_issue_type: Option<&str>,
        diagnostics: &str,
    ) -> ResponseTemplate {
        let body = outcome_body(issue_code, tx_issue_type, diagnostics);
        ResponseTemplate::new(status)
            .insert_header("Content-Type", FHIR_JSON)
            .set_body_string(body.to_string())
    }

    /// Returns the `batch-response` `Bundle` of a batch, one entry per
    /// request in the order they were sent.
    ///
    /// Each entry carries "the status code returned by processing this entry"
    /// as an HTTP status line and the resource that request would have
    /// answered on its own
    /// (<https://hl7.org/fhir/R4/bundle-definitions.html#Bundle.entry.response.status>).
    #[must_use]
    pub fn batch_response(entries: &[(&str, serde_json::Value)]) -> ResponseTemplate {
        let entry: Vec<serde_json::Value> = entries
            .iter()
            .map(|(status, resource)| {
                serde_json::json!({
                    "resource": resource,
                    "response": {"status": status},
                })
            })
            .collect();
        let body = serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch-response",
            "entry": entry,
        });
        ResponseTemplate::new(200)
            .insert_header("Content-Type", FHIR_JSON)
            .set_body_string(body.to_string())
    }

    /// Returns a `Parameters` resource carrying `parameters`, the body an
    /// operation answers with.
    #[must_use]
    pub fn parameters_resource(parameters: &[serde_json::Value]) -> serde_json::Value {
        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        })
    }

    /// Returns a `200` carrying `parameters` as a FHIR `Parameters`.
    fn fhir_parameters(parameters: &[serde_json::Value]) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .insert_header("Content-Type", FHIR_JSON)
            .set_body_string(parameters_resource(parameters).to_string())
    }
}
