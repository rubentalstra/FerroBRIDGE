// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A parameter reads the primitives that share its scalar, in either
//! direction (#352).

use fhir_types::r4b::operations::value_set_expand::ValueSetExpandRequest;
use fhir_types::r4b::operations::value_set_validate_code::ValueSetValidateCodeRequest;
use fhir_types::r4b::parameters::{Parameters, ParametersParameter, ParametersParameterValue};

/// The value the terminology ecosystem suite sends for `system-version`.
const PINNED: &str = "http://snomed.info/sct|http://snomed.info/sct/900000000000207008";

fn system_version(value: ParametersParameterValue) -> Parameters {
    Parameters {
        parameter: vec![ParametersParameter {
            name: "system-version".into(),
            value: Some(value),
            ..Default::default()
        }],
        ..Default::default()
    }
}

// `canonical` and `uri` are distinct types that "are never substituted for
// each other" (<https://hl7.org/fhir/R4B/datatypes.html#uri>), and no
// clause requires a server to refuse the wider spelling; the GET form
// carries no type marker at all
// (<https://hl7.org/fhir/R4B/operations.html>), so the declared type is
// applied by parsing either way.
#[test]
fn a_canonical_parameter_reads_both_spellings_on_expand() {
    let canonical = ValueSetExpandRequest::from_parameters(&system_version(
        ParametersParameterValue::Canonical(PINNED.into()),
    ))
    .expect("the declared spelling reads");
    let uri = ValueSetExpandRequest::from_parameters(&system_version(
        ParametersParameterValue::Uri(PINNED.into()),
    ))
    .expect("the wider spelling reads");
    assert_eq!(canonical.system_version, uri.system_version);
    assert_eq!(
        canonical
            .system_version
            .first()
            .and_then(|v| v.value.as_deref()),
        Some(PINNED)
    );
}

#[test]
fn a_canonical_parameter_reads_both_spellings_on_validate_code() {
    let canonical = ValueSetValidateCodeRequest::from_parameters(&system_version(
        ParametersParameterValue::Canonical(PINNED.into()),
    ))
    .expect("the declared spelling reads");
    let uri = ValueSetValidateCodeRequest::from_parameters(&system_version(
        ParametersParameterValue::Uri(PINNED.into()),
    ))
    .expect("the wider spelling reads");
    assert_eq!(canonical.system_version, uri.system_version);
}
