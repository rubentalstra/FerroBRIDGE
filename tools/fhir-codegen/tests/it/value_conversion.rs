// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

use fhir_types::codec::{
    DecodeErrorKind, Json, Number, Object, Path, Value, ValueConversionError, expect_object,
};

/// The codec value of `lexical`, which every caller below writes as a JSON
/// number.
fn number(lexical: &str) -> Value {
    Value::Number(lexical.parse::<Number>().expect("a JSON number"))
}

/// Converts `value` under a path rooted at `root`.
fn convert(value: Value, root: &str) -> Result<serde_json::Value, ValueConversionError> {
    value.into_serde_json(&mut Path::root(root))
}

/// An element holding a `valueQuantity` whose value is `lexical`.
fn quantity(lexical: &str) -> Value {
    let mut value = Object::new();
    value.insert(String::from("value"), number(lexical));
    let mut quantity = Object::new();
    quantity.insert(String::from("valueQuantity"), Value::Object(value));
    Value::Object(quantity)
}

#[test]
fn a_number_serde_json_writes_back_unchanged_converts() {
    // Every form here is the text serde_json writes for the same number, so
    // the conversion keeps the document's own digits
    // (https://hl7.org/fhir/R4/datatypes.html#decimal).
    for lexical in ["1", "-7", "1.5", "0.0", "100", "1.0"] {
        let converted = convert(number(lexical), "Observation").expect("an exact number converts");
        assert_eq!(converted.to_string(), lexical);
    }
}

#[test]
fn a_decimal_with_a_trailing_zero_is_refused_with_its_path() {
    // FHIR regards 0.010 as different from 0.01 and keeps the original
    // precision (https://hl7.org/fhir/R4/datatypes.html#decimal), so a
    // serde_json that rounds 1.50 to 1.5 never sees the value.
    let error = convert(quantity("1.50"), "Observation").expect_err("a rounded decimal");
    assert_eq!(
        error,
        ValueConversionError::LossyNumber {
            path: String::from("Observation.valueQuantity.value"),
            lexical: String::from("1.50"),
            rendered: String::from("1.5"),
        }
    );
}

#[test]
fn an_exponent_form_is_refused() {
    let error = convert(number("1e3"), "Observation").expect_err("an exponent form");
    assert_eq!(
        error,
        ValueConversionError::LossyNumber {
            path: String::from("Observation"),
            lexical: String::from("1e3"),
            rendered: String::from("1000.0"),
        }
    );
}

#[test]
fn a_decimal_past_f64_precision_is_refused() {
    let lexical = "0.1000000000000000055511151231257827";
    let error = convert(quantity(lexical), "Observation").expect_err("a decimal past f64");
    assert_eq!(
        error,
        ValueConversionError::LossyNumber {
            path: String::from("Observation.valueQuantity.value"),
            lexical: String::from(lexical),
            rendered: String::from("0.1"),
        }
    );
}

#[test]
fn a_number_serde_json_cannot_hold_is_refused() {
    // serde_json reads 1e400 as a raw value and refuses it as a number, so the
    // conversion reports it rather than dropping the element.
    let error = convert(quantity("1e400"), "Observation").expect_err("a number out of range");
    assert_eq!(
        error,
        ValueConversionError::UnrepresentableNumber {
            path: String::from("Observation.valueQuantity.value"),
            lexical: String::from("1e400"),
        }
    );
}

#[test]
fn an_array_element_carries_its_index_in_the_path() {
    let mut observation = Object::new();
    observation.insert(
        String::from("component"),
        Value::Array(vec![quantity("1"), quantity("2.50")]),
    );
    let error = convert(Value::Object(observation), "Observation").expect_err("a rounded decimal");
    assert_eq!(
        error,
        ValueConversionError::LossyNumber {
            path: String::from("Observation.component[1].valueQuantity.value"),
            lexical: String::from("2.50"),
            rendered: String::from("2.5"),
        }
    );
}

#[test]
fn a_nested_document_round_trips_through_serde_json() {
    let text = r#"{"resourceType":"Observation","status":"final","component":[{"valueQuantity":{"value":1.5,"unit":"mg"}},{"valueString":"x"}],"_status":{"id":"s1"},"absent":null,"flag":true}"#;
    let original: Value = serde_json::from_str(text).expect("the fixture is JSON");
    let converted = original
        .to_serde_json(&mut Path::root("Observation"))
        .expect("every number is exact");
    assert_eq!(Value::from_serde_json(converted), original);
}

#[test]
fn a_serde_json_number_arrives_in_the_text_serde_json_holds() {
    let value = Value::from_serde_json(serde_json::json!({"value": 1.5}));
    assert_eq!(value.get("value"), Some(&number("1.5")));
    let whole = Value::from_serde_json(serde_json::json!({"value": 3}));
    assert_eq!(whole.get("value"), Some(&number("3")));
}

#[test]
fn the_strict_codec_still_refuses_a_document_from_serde_json() {
    // The conversion is a transport, so the codec's refusals hold on a
    // document that arrives as a serde_json::Value
    // (https://hl7.org/fhir/R4/json.html).
    let coding = Value::from_serde_json(serde_json::json!({"code": "a", "bogus": 1}));
    let mut path = Path::root("Coding");
    let unknown = fhir_types::r4::coding::Coding::from_json(
        expect_object(&coding, &path).expect("an object"),
        &mut path,
    )
    .expect_err("an unknown property");
    assert_eq!(unknown.kind, DecodeErrorKind::UnknownProperty);
    assert_eq!(unknown.path, "Coding.bogus");

    let parameters = Value::from_serde_json(serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "x", "valueString": "a", "valueCode": "b"}],
    }));
    let mut path = Path::root("Parameters");
    let duplicate = fhir_types::r4::parameters::Parameters::from_json(
        expect_object(&parameters, &path).expect("an object"),
        &mut path,
    )
    .expect_err("two forms of one choice element");
    assert_eq!(duplicate.kind, DecodeErrorKind::DuplicateChoice);
}
