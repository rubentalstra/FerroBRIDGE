// SPDX-FileCopyrightText: Ruben Talstra
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

/// The FHIR R4 `Parameters` the terminology operations return.
pub mod terminology {
    use wiremock::ResponseTemplate;

    /// The FHIR JSON media type of R4.
    const FHIR_JSON: &str = "application/fhir+json";

    /// Returns the `Parameters` of `CodeSystem/$lookup`.
    ///
    /// `name` and `display` are the two mandatory out parameters
    /// (`OperationDefinition-CodeSystem-lookup`, R4).
    #[must_use]
    pub fn lookup(name: &str, display: &str, version: Option<&str>) -> ResponseTemplate {
        let mut parameters = vec![
            serde_json::json!({"name": "name", "valueString": name}),
            serde_json::json!({"name": "display", "valueString": display}),
        ];
        if let Some(version) = version {
            parameters.push(serde_json::json!({"name": "version", "valueString": version}));
        }
        fhir_parameters(&parameters)
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
        fhir_parameters(&[
            serde_json::json!({"name": "result", "valueBoolean": true}),
            serde_json::json!({
                "name": "match",
                "part": [
                    {"name": "equivalence", "valueCode": equivalence},
                    {"name": "concept", "valueCoding": {
                        "system": target_system,
                        "code": target_code,
                        "display": target_display,
                    }},
                ],
            }),
        ])
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
        let mut parameters = vec![serde_json::json!({"name": "result", "valueBoolean": result})];
        if let Some(display) = display {
            parameters.push(serde_json::json!({"name": "display", "valueString": display}));
        }
        if let Some(message) = message {
            parameters.push(serde_json::json!({"name": "message", "valueString": message}));
        }
        fhir_parameters(&parameters)
    }

    /// Returns a `200` carrying `parameters` as a FHIR R4 `Parameters`.
    fn fhir_parameters(parameters: &[serde_json::Value]) -> ResponseTemplate {
        let body = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": parameters,
        });
        ResponseTemplate::new(200)
            .insert_header("Content-Type", FHIR_JSON)
            .set_body_string(body.to_string())
    }
}
