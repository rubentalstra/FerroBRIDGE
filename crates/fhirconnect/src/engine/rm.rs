// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reference-model values the data-type cells read and write.
//!
//! Each [`crate::engine::lens::Lens`] is typed in one openEHR class, and the
//! interpreter holds whichever class the template node carries, so [`RmValue`]
//! is the sum of the classes the FHIR round trip touches. It also carries the
//! two wires a composition is reached through: the canonical JSON one value
//! reads back as, and the FLAT keys a value is written under (openEHR
//! ITS-REST 1.1.0, Simplified Formats).
//!
//! The FLAT decomposition is not a second model of a data value. It is the
//! key spelling of the same value, one `|suffix` or sub-path per part, taken
//! from the Simplified Formats attribute tables, and the builder of
//! `openehr-its` rebuilds the reference-model instance from it.

use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentifiedData;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;
use openehr_rm::v1_2::data_types::quantity::dv_proportion::DvProportion;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_rm::v1_2::data_types::text::dv_text::DvText;
use openehr_rm::v1_2::data_types::text::dv_text::DvTextData;
use openehr_rm::v1_2::data_types::text::term_mapping::TermMapping;
use serde_json::Value;

/// One openEHR value a data-type cell produced or consumed.
#[derive(Debug, Clone, PartialEq)]
pub enum RmValue {
    /// A `DV_TEXT`.
    Text(DvTextData),
    /// A `DV_CODED_TEXT`.
    CodedText(Box<DvCodedText>),
    /// A `CODE_PHRASE`.
    CodePhrase(CodePhrase),
    /// A `DV_DATE_TIME`.
    DateTime(Box<DvDateTime>),
    /// A `DV_INTERVAL<DV_DATE_TIME>`.
    DateTimeInterval(Box<DvInterval<DvDateTime>>),
    /// A `PARTY_IDENTIFIED`, the composer and every participation.
    Party(PartyIdentifiedData),
    /// A `DV_IDENTIFIER`.
    Identifier(DvIdentifier),
    /// A `DV_PROPORTION`.
    Proportion(Box<DvProportion>),
}

/// Why a reference-model value could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RmError {
    /// The engine holds no cell for the class the template node carries.
    #[error("no data-type cell carries the openEHR class {rm_type}")]
    UnknownClass {
        /// The class the Web Template node names.
        rm_type: String,
    },
    /// The canonical JSON does not decode as the class the node names.
    #[error("the value at {node} does not read as {rm_type}: {reason}")]
    Decode {
        /// The node the value was read from.
        node: String,
        /// The class the node names.
        rm_type: String,
        /// What the decoder refused.
        reason: String,
    },
    /// The value has no canonical JSON form.
    #[error("the {rm_type} value has no canonical JSON form: {reason}")]
    Encode {
        /// The class that refused.
        rm_type: &'static str,
        /// What the encoder refused.
        reason: String,
    },
}

/// One FLAT key part of a reference-model value.
///
/// The key is the node's own FLAT id, then [`Part::sub_path`], then `|` and
/// [`Part::datum`] where the part is a datum suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    sub_path: Vec<String>,
    datum: Option<&'static str>,
    value: Value,
}

impl Part {
    /// Returns the value-internal family the part sits under.
    #[must_use]
    pub fn sub_path(&self) -> &[String] {
        &self.sub_path
    }

    /// Returns the datum suffix, `None` for the node's own value.
    #[must_use]
    pub const fn datum(&self) -> Option<&'static str> {
        self.datum
    }

    /// Returns the value the part carries.
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }

    /// Creates a part at the node itself, under `datum`.
    fn datum_of(datum: &'static str, value: impl Into<Value>) -> Self {
        Self {
            sub_path: Vec::new(),
            datum: Some(datum),
            value: value.into(),
        }
    }

    /// Creates a part at the node itself, with no datum suffix.
    fn bare(value: impl Into<Value>) -> Self {
        Self {
            sub_path: Vec::new(),
            datum: None,
            value: value.into(),
        }
    }

    /// Returns this part under a value-internal family.
    fn under(mut self, segments: &[String]) -> Self {
        let mut path = segments.to_vec();
        path.append(&mut self.sub_path);
        self.sub_path = path;
        self
    }
}

impl RmValue {
    /// Returns the openEHR class the value carries.
    #[must_use]
    pub const fn rm_type(&self) -> &'static str {
        match *self {
            Self::Text(_) => "DV_TEXT",
            Self::CodedText(_) => "DV_CODED_TEXT",
            Self::CodePhrase(_) => "CODE_PHRASE",
            Self::DateTime(_) => "DV_DATE_TIME",
            Self::DateTimeInterval(_) => "DV_INTERVAL",
            Self::Party(_) => "PARTY_IDENTIFIED",
            Self::Identifier(_) => "DV_IDENTIFIER",
            Self::Proportion(_) => "DV_PROPORTION",
        }
    }

    /// Reads the canonical JSON of one node as the class the node names.
    ///
    /// A `DV_TEXT` node may hold a `DV_CODED_TEXT`, which the reference model
    /// admits as a subtype
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/data_types.html#_dv_coded_text_class>),
    /// so the discriminator in the document decides between the two.
    ///
    /// # Errors
    ///
    /// Returns [`RmError::UnknownClass`] for a class no cell carries and
    /// [`RmError::Decode`] when the document does not decode as it.
    pub fn from_canonical(rm_type: &str, node: &str, value: &Value) -> Result<Self, RmError> {
        let decode = |result: Result<Self, serde_json::Error>| {
            result.map_err(|source| RmError::Decode {
                node: String::from(node),
                rm_type: String::from(rm_type),
                reason: source.to_string(),
            })
        };
        match rm_type {
            "DV_TEXT" | "DV_CODED_TEXT" => decode(
                serde_json::from_value::<DvText>(value.clone()).map(|text| match text {
                    DvText::DvCodedText(coded) => Self::CodedText(Box::new(coded)),
                    DvText::DvText(plain) => Self::Text(plain),
                }),
            ),
            "CODE_PHRASE" => decode(serde_json::from_value(value.clone()).map(Self::CodePhrase)),
            "DV_DATE_TIME" => decode(
                serde_json::from_value(value.clone()).map(|date| Self::DateTime(Box::new(date))),
            ),
            "DV_INTERVAL" => decode(
                serde_json::from_value(value.clone())
                    .map(|interval| Self::DateTimeInterval(Box::new(interval))),
            ),
            "PARTY_IDENTIFIED" | "PARTY_PROXY" => {
                decode(serde_json::from_value(value.clone()).map(Self::Party))
            }
            "DV_IDENTIFIER" => decode(serde_json::from_value(value.clone()).map(Self::Identifier)),
            "DV_PROPORTION" => decode(
                serde_json::from_value(value.clone())
                    .map(|proportion| Self::Proportion(Box::new(proportion))),
            ),
            other => Err(RmError::UnknownClass {
                rm_type: String::from(other),
            }),
        }
    }

    /// Returns the FLAT parts one value is written under.
    ///
    /// The suffixes are the ones the Simplified Formats attribute tables fix
    /// per class, and a part the class leaves unset is left out.
    ///
    /// # Errors
    ///
    /// Returns [`RmError::Encode`] when a numeric attribute has no JSON form.
    pub fn parts(&self) -> Result<Vec<Part>, RmError> {
        match *self {
            Self::Text(ref text) => Ok(text_parts(text)),
            Self::CodedText(ref coded) => Ok(coded_parts(coded)),
            Self::CodePhrase(ref code) => Ok(code_phrase_parts(code)),
            Self::DateTime(ref date) => Ok(date_time_parts(date)),
            Self::DateTimeInterval(ref interval) => Ok(interval_parts(interval)),
            Self::Party(ref party) => Ok(party_parts(party)),
            Self::Identifier(ref identifier) => Ok(identifier_parts(identifier, &[])),
            Self::Proportion(ref proportion) => proportion_parts(proportion),
        }
    }
}

/// The parts of a `DV_TEXT` (Simplified Formats, §`DV_TEXT`).
fn text_parts(text: &DvTextData) -> Vec<Part> {
    let mut parts = vec![Part::datum_of("value", text.value.clone())];
    if let Some(ref formatting) = text.formatting {
        parts.push(Part::datum_of("formatting", formatting.clone()));
    }
    parts.extend(mapping_parts(text.mappings.as_deref().unwrap_or_default()));
    parts
}

/// The parts of a `DV_CODED_TEXT` (Simplified Formats, §`DV_CODED_TEXT`).
fn coded_parts(coded: &DvCodedText) -> Vec<Part> {
    let mut parts = vec![
        Part::datum_of("value", coded.value.clone()),
        Part::datum_of("code", coded.defining_code.code_string.clone()),
        Part::datum_of(
            "terminology",
            coded.defining_code.terminology_id.value.clone(),
        ),
    ];
    if let Some(ref preferred) = coded.defining_code.preferred_term {
        parts.push(Part::datum_of("preferred_term", preferred.clone()));
    }
    if let Some(ref formatting) = coded.formatting {
        parts.push(Part::datum_of("formatting", formatting.clone()));
    }
    parts.extend(mapping_parts(coded.mappings.as_deref().unwrap_or_default()));
    parts
}

/// The parts of a `CODE_PHRASE` (Simplified Formats, §`CODE_PHRASE`).
fn code_phrase_parts(code: &CodePhrase) -> Vec<Part> {
    let mut parts = vec![
        Part::datum_of("code", code.code_string.clone()),
        Part::datum_of("terminology", code.terminology_id.value.clone()),
    ];
    if let Some(ref preferred) = code.preferred_term {
        parts.push(Part::datum_of("preferred_term", preferred.clone()));
    }
    parts
}

/// The parts of a `DV_DATE_TIME`.
///
/// The value carries no suffix of its own, so the date and time text is the
/// node's own value and reaches the builder byte for byte.
fn date_time_parts(date: &DvDateTime) -> Vec<Part> {
    let mut parts = vec![Part::bare(date.value.clone())];
    if let Some(ref status) = date.magnitude_status {
        parts.push(Part::datum_of("magnitude_status", status.clone()));
    }
    parts
}

/// The parts of a `DV_INTERVAL<DV_DATE_TIME>` (Simplified Formats,
/// §`DV_INTERVAL`).
fn interval_parts(interval: &DvInterval<DvDateTime>) -> Vec<Part> {
    let mut parts = Vec::new();
    if let Some(ref lower) = interval.lower {
        parts.push(Part::bare(lower.value.clone()).under(&[String::from("lower")]));
    }
    if let Some(ref upper) = interval.upper {
        parts.push(Part::bare(upper.value.clone()).under(&[String::from("upper")]));
    }
    parts.push(Part::datum_of("lower_unbounded", interval.lower_unbounded));
    parts.push(Part::datum_of("upper_unbounded", interval.upper_unbounded));
    parts.push(Part::datum_of("lower_included", interval.lower_included));
    parts.push(Part::datum_of("upper_included", interval.upper_included));
    parts
}

/// The parts of a `PARTY_IDENTIFIED` (Simplified Formats,
/// §`PARTY_IDENTIFIED`).
fn party_parts(proxy: &PartyIdentifiedData) -> Vec<Part> {
    let mut parts = Vec::new();
    if let Some(ref name) = proxy.name {
        parts.push(Part::datum_of("name", name.clone()));
    }
    for (index, identifier) in proxy
        .identifiers
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        parts.extend(identifier_parts(
            identifier,
            &[format!("_identifier:{index}")],
        ));
    }
    parts
}

/// The parts of a `DV_IDENTIFIER` (Simplified Formats, §`DV_IDENTIFIER`).
fn identifier_parts(identifier: &DvIdentifier, prefix: &[String]) -> Vec<Part> {
    let mut parts = vec![Part::datum_of("id", identifier.id.clone())];
    if let Some(ref issuer) = identifier.issuer {
        parts.push(Part::datum_of("issuer", issuer.clone()));
    }
    if let Some(ref assigner) = identifier.assigner {
        parts.push(Part::datum_of("assigner", assigner.clone()));
    }
    if let Some(ref kind) = identifier.r#type {
        parts.push(Part::datum_of("type", kind.clone()));
    }
    parts
        .into_iter()
        .map(|part| part.under(prefix))
        .collect::<Vec<Part>>()
}

/// The parts of a `DV_PROPORTION` (Simplified Formats, §`DV_PROPORTION`).
fn proportion_parts(proportion: &DvProportion) -> Result<Vec<Part>, RmError> {
    let number = |value: f64| {
        serde_json::Number::from_f64(value).ok_or_else(|| RmError::Encode {
            rm_type: "DV_PROPORTION",
            reason: format!("{value} is not a finite JSON number"),
        })
    };
    let mut parts = vec![
        Part::datum_of("numerator", Value::Number(number(proportion.numerator)?)),
        Part::datum_of(
            "denominator",
            Value::Number(number(proportion.denominator)?),
        ),
        Part::datum_of("type", proportion.r#type),
    ];
    if let Some(precision) = proportion.precision {
        parts.push(Part::datum_of("precision", precision));
    }
    Ok(parts)
}

/// The `_mapping:i` family of a text value (Simplified Formats,
/// §`TERM_MAPPING`).
fn mapping_parts(mappings: &[TermMapping]) -> Vec<Part> {
    let mut parts = Vec::new();
    for (index, mapping) in mappings.iter().enumerate() {
        let family = [format!("_mapping:{index}")];
        parts.push(Part::datum_of("match", mapping.r#match.to_string()).under(&family));
        let target = [family.concat(), String::from("target")];
        parts.extend(
            code_phrase_parts(&mapping.target)
                .into_iter()
                .map(|part| part.under(&target)),
        );
    }
    parts
}

/// Returns the identifiers a party carries, as the container the model wants.
///
/// `NonEmptyVec` is the reference model's own container for an optional
/// non-empty list, so an empty list is the absent one rather than a present
/// list of nothing.
#[must_use]
pub fn identifiers(values: Vec<DvIdentifier>) -> Option<NonEmptyVec<DvIdentifier>> {
    NonEmptyVec::new(values).ok()
}

#[cfg(test)]
mod tests {
    use super::Part;
    use super::RmValue;
    use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
    use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;

    /// Renders one part as the key tail it writes under.
    fn key(part: &Part) -> String {
        let mut rendered = part.sub_path().join("/");
        if let Some(datum) = part.datum() {
            rendered.push('|');
            rendered.push_str(datum);
        }
        rendered
    }

    #[test]
    fn a_code_phrase_writes_the_three_suffixes_its_table_names() {
        let value = RmValue::CodePhrase(CodePhrase {
            terminology_id: TerminologyId {
                value: String::from("local"),
            },
            code_string: String::from("at0017"),
            preferred_term: Some(String::from("Working")),
        });
        let parts = value.parts().expect("a code phrase has FLAT parts");
        let keys: Vec<String> = parts.iter().map(key).collect();
        assert_eq!(keys, ["|code", "|terminology", "|preferred_term"]);
    }

    #[test]
    fn a_date_time_writes_its_text_as_the_node_value() {
        let value = RmValue::from_canonical(
            "DV_DATE_TIME",
            "/context/start_time",
            &serde_json::json!({"_type": "DV_DATE_TIME", "value": "2026-09-12T10:00:00+02:00"}),
        )
        .expect("the canonical form reads back");
        let parts = value.parts().expect("a date and time has one FLAT part");
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts
                .first()
                .map(Part::value)
                .and_then(serde_json::Value::as_str),
            Some("2026-09-12T10:00:00+02:00"),
            "the offset survives byte for byte"
        );
    }

    #[test]
    fn a_class_no_cell_carries_names_itself() {
        let error = RmValue::from_canonical("DV_MULTIMEDIA", "/x", &serde_json::Value::Null)
            .expect_err("no cell carries DV_MULTIMEDIA");
        assert!(
            error.to_string().contains("DV_MULTIMEDIA"),
            "the refusal names the class: {error}"
        );
    }

    #[test]
    fn a_coded_text_writes_its_term_mappings_as_a_family() {
        let value = RmValue::from_canonical(
            "DV_CODED_TEXT",
            "/x",
            &serde_json::json!({
                "_type": "DV_CODED_TEXT",
                "value": "Synthetic problem one",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                    "code_string": "at0001"
                },
                "mappings": [{
                    "_type": "TERM_MAPPING",
                    "match": "=",
                    "target": {
                        "_type": "CODE_PHRASE",
                        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "SNOMED-CT"},
                        "code_string": "73211009"
                    }
                }]
            }),
        )
        .expect("the canonical form reads back");
        let parts = value.parts().expect("a coded text has FLAT parts");
        let keys: Vec<String> = parts.iter().map(key).collect();
        assert!(
            keys.contains(&String::from("_mapping:0/target|code")),
            "the mapping target is a sub-path family: {keys:?}"
        );
        assert!(
            keys.contains(&String::from("_mapping:0|match")),
            "the match is a suffix on the family: {keys:?}"
        );
    }
}
