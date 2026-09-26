// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The generated `Parameters` conversions: exact to the declared set.

use fhir_types::operation::ParametersError;
use fhir_types::r4b::coding::Coding;
use fhir_types::r4b::operations::code_system_lookup::{
    CodeSystemLookupResponse, CodeSystemLookupResponseDesignation, CodeSystemLookupResponseProperty,
};
use fhir_types::r4b::operations::code_system_validate_code::CodeSystemValidateCodeRequest;
use fhir_types::r4b::parameters::{Parameters, ParametersParameter, ParametersParameterValue};

fn param(name: &str, value: ParametersParameterValue) -> ParametersParameter {
    ParametersParameter {
        name: name.into(),
        value: Some(value),
        ..Default::default()
    }
}

#[test]
fn a_lookup_response_round_trips_through_parameters() {
    let response = CodeSystemLookupResponse {
        name: "SNOMED CT".into(),
        version: Some("http://snomed.info/sct/900000000000207008/version/20260101".into()),
        display: "Heart".into(),
        designation: vec![CodeSystemLookupResponseDesignation {
            language: Some("nl".into()),
            r#use: Some(Coding {
                system: Some("http://snomed.info/sct".into()),
                code: Some("900000000000013009".into()),
                ..Default::default()
            }),
            value: "Hart".into(),
        }],
        property: vec![CodeSystemLookupResponseProperty {
            code: "inactive".into(),
            value: Some(ParametersParameterValue::Boolean(false.into())),
            description: None,
            subproperty: Vec::new(),
        }],
        code: None,
        system: None,
        r#abstract: None,
    };
    let parameters = response.clone().into_parameters();
    let names: Vec<&str> = parameters
        .parameter
        .iter()
        .filter_map(|p| p.name.value.as_deref())
        .collect();
    assert_eq!(
        names,
        ["name", "version", "display", "designation", "property"]
    );
    assert_eq!(
        parameters.parameter[3].part.len(),
        3,
        "language, use, value"
    );
    let back = CodeSystemLookupResponse::from_parameters(&parameters).expect("reads back");
    assert_eq!(back, response);
}

#[test]
fn r4b_validate_code_refuses_the_undeclared_system_parameter() {
    // The R4B definition declares `url`, not `system`, although its prose
    // says "code+system": strict means exactly the declared set.
    let parameters = Parameters {
        parameter: vec![
            param(
                "system",
                ParametersParameterValue::Uri("http://snomed.info/sct".into()),
            ),
            param("code", ParametersParameterValue::Code("80146002".into())),
        ],
        ..Default::default()
    };
    assert_eq!(
        CodeSystemValidateCodeRequest::from_parameters(&parameters),
        Err(ParametersError::Undeclared {
            operation: "CodeSystem/$validate-code",
            name: String::from("system"),
        })
    );
}

#[test]
fn repeated_missing_unnamed_and_wrongly_typed_parameters_are_refused() {
    let twice = Parameters {
        parameter: vec![
            param("code", ParametersParameterValue::Code("1".into())),
            param("code", ParametersParameterValue::Code("2".into())),
        ],
        ..Default::default()
    };
    assert_eq!(
        CodeSystemValidateCodeRequest::from_parameters(&twice),
        Err(ParametersError::Repeated {
            operation: "CodeSystem/$validate-code",
            name: "code",
        })
    );
    let missing = Parameters {
        parameter: vec![param(
            "display",
            ParametersParameterValue::String("Heart".into()),
        )],
        ..Default::default()
    };
    assert_eq!(
        CodeSystemLookupResponse::from_parameters(&missing),
        Err(ParametersError::Missing {
            operation: "CodeSystem/$lookup",
            name: "name",
        })
    );
    let unnamed = Parameters {
        parameter: vec![ParametersParameter::default()],
        ..Default::default()
    };
    assert_eq!(
        CodeSystemValidateCodeRequest::from_parameters(&unnamed),
        Err(ParametersError::Unnamed {
            operation: "CodeSystem/$validate-code",
        })
    );
    let wrong = Parameters {
        parameter: vec![param(
            "code",
            ParametersParameterValue::Boolean(true.into()),
        )],
        ..Default::default()
    };
    assert_eq!(
        CodeSystemValidateCodeRequest::from_parameters(&wrong),
        Err(ParametersError::WrongType {
            operation: "CodeSystem/$validate-code",
            name: "code",
            expected: "Code",
        })
    );
    let valueless = Parameters {
        parameter: vec![ParametersParameter {
            name: "code".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(
        CodeSystemValidateCodeRequest::from_parameters(&valueless),
        Err(ParametersError::MissingValue {
            operation: "CodeSystem/$validate-code",
            name: "code",
        })
    );
}

#[test]
fn a_part_error_names_the_dotted_path() {
    let parameters = Parameters {
        parameter: vec![
            param("name", ParametersParameterValue::String("x".into())),
            param("display", ParametersParameterValue::String("y".into())),
            ParametersParameter {
                name: "designation".into(),
                part: vec![param(
                    "colour",
                    ParametersParameterValue::String("red".into()),
                )],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert_eq!(
        CodeSystemLookupResponse::from_parameters(&parameters),
        Err(ParametersError::Undeclared {
            operation: "CodeSystem/$lookup",
            name: String::from("designation.colour"),
        })
    );
}
