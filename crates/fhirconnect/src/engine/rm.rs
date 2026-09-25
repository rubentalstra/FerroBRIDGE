// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reference-model values the data-type cells read and write.
//!
//! Each [`crate::engine::lens::Lens`] is typed in one openEHR class, and the
//! interpreter holds whichever class the template node carries, so [`RmValue`]
//! is the sum of the classes the FHIR round trip touches. A value crosses both
//! wires a composition is reached through as its canonical JSON, read and
//! written by the ITS-JSON entry points of `openehr-its`: the CDR serves it,
//! and the FLAT builder takes it whole under the `|raw` suffix (openEHR
//! ITS-REST 1.1.0, Simplified Formats, master04 §Raw canonical JSON).
//!
//! Which tail below a node a value carries is the reference-model attribute
//! model `openehr-rm` generates from the RM BMM, so nothing here restates the
//! attributes of a class.

use openehr_base::containers::NonEmptyVec;
use openehr_its::json::JsonParseError;
use openehr_its::json::from_canonical_value;
use openehr_its::json::to_canonical_value;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentifiedData;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;
use openehr_rm::v1_2::data_types::quantity::dv_proportion::DvProportion;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_rm::v1_2::data_types::text::dv_text::DvText;
use openehr_rm::v1_2::data_types::text::dv_text::DvTextData;
use openehr_rm::v1_2::model;
use openehr_rm::v1_2::model::Container;
use openehr_sdt::flat::path::Segment;
use serde_json::Value;

use crate::resolve::derive;

/// The suffix a whole canonical value travels under in a FLAT key.
///
/// Simplified Formats, master04 §Raw canonical JSON: the value "must include
/// the `_type` property", which the ITS-JSON writer puts first.
pub const RAW: &str = "raw";

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
    #[error("the value at {node} does not read as {rm_type}")]
    Decode {
        /// The node the value was read from.
        node: String,
        /// The class the node names.
        rm_type: String,
        /// What the strict ITS-JSON reader refused, with its JSON path.
        #[source]
        source: JsonParseError,
    },
    /// A scalar attribute's text does not read as the attribute's type.
    #[error("the {rm_type} attribute at {node} takes {expected}, and `{text}` is none")]
    Scalar {
        /// The node the attribute is below.
        node: String,
        /// The class that owns the attribute.
        rm_type: String,
        /// The text the FHIR element carried.
        text: String,
        /// What the attribute's declared type takes.
        expected: &'static str,
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
    /// so the discriminator in the document decides between the two. The
    /// reader is the strict ITS-JSON one of `openehr-its`.
    ///
    /// # Errors
    ///
    /// Returns [`RmError::UnknownClass`] for a class no cell carries and
    /// [`RmError::Decode`] when the document does not decode as it.
    pub fn from_canonical(rm_type: &str, node: &str, value: &Value) -> Result<Self, RmError> {
        let refuse = |source: JsonParseError| RmError::Decode {
            node: String::from(node),
            rm_type: String::from(rm_type),
            source,
        };
        match rm_type {
            "DV_TEXT" | "DV_CODED_TEXT" => from_canonical_value::<DvText>(value)
                .map(|text| match text {
                    DvText::DvCodedText(coded) => Self::CodedText(Box::new(coded)),
                    DvText::DvText(plain) => Self::Text(plain),
                })
                .map_err(refuse),
            "CODE_PHRASE" => from_canonical_value(value)
                .map(Self::CodePhrase)
                .map_err(refuse),
            "DV_DATE_TIME" => from_canonical_value(value)
                .map(|date| Self::DateTime(Box::new(date)))
                .map_err(refuse),
            "DV_INTERVAL" => from_canonical_value(value)
                .map(|interval| Self::DateTimeInterval(Box::new(interval)))
                .map_err(refuse),
            "PARTY_IDENTIFIED" | "PARTY_PROXY" => {
                from_canonical_value(value).map(Self::Party).map_err(refuse)
            }
            "DV_IDENTIFIER" => from_canonical_value(value)
                .map(Self::Identifier)
                .map_err(refuse),
            "DV_PROPORTION" => from_canonical_value(value)
                .map(|proportion| Self::Proportion(Box::new(proportion)))
                .map_err(refuse),
            other => Err(RmError::UnknownClass {
                rm_type: String::from(other),
            }),
        }
    }

    /// Returns the canonical JSON of the value, `_type` first.
    ///
    /// # Errors
    ///
    /// Returns [`RmError::Encode`] for a `DV_PROPORTION` whose numerator or
    /// denominator is no finite number, which JSON has no form for
    /// (<https://www.rfc-editor.org/rfc/rfc8259#section-6>).
    pub fn to_canonical(&self) -> Result<Value, RmError> {
        let encoded = match *self {
            Self::Text(ref text) => to_canonical_value(text),
            Self::CodedText(ref coded) => to_canonical_value(coded.as_ref()),
            Self::CodePhrase(ref code) => to_canonical_value(code),
            Self::DateTime(ref date) => to_canonical_value(date.as_ref()),
            Self::DateTimeInterval(ref interval) => to_canonical_value(interval.as_ref()),
            Self::Party(ref party) => to_canonical_value(party),
            Self::Identifier(ref identifier) => to_canonical_value(identifier),
            Self::Proportion(ref proportion) => {
                for number in [proportion.numerator, proportion.denominator] {
                    if !number.is_finite() {
                        return Err(RmError::Encode {
                            rm_type: "DV_PROPORTION",
                            reason: format!("{number} is not a finite JSON number"),
                        });
                    }
                }
                to_canonical_value(proportion.as_ref())
            }
        };
        Ok(encoded)
    }
}

/// What the attribute a tail ends on holds.
///
/// A tail below a template node names either one scalar attribute of the
/// node's data value or one data value nested in it; the whole value reaches
/// the wire under `|raw`, so every attribute the reference model gives the
/// value is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carried {
    /// A string attribute.
    Text,
    /// A real-valued attribute.
    Real,
    /// An integer attribute.
    Integer,
    /// A boolean attribute.
    Boolean,
    /// A data value of the tail's leaf class, read and written by its cell.
    Value,
    /// A data value of the tail's leaf class that is an attribute of a
    /// structural node, which FLAT writes as the `_`-prefixed family of the
    /// node (`_provider` on an `ENTRY`) and the engine writes through
    /// [`crate::engine::family::provider`].
    Family,
}

/// Returns the class a tail below a node of `rm_type` is written as, and what
/// the tail's attribute holds, `None` when the engine cannot carry it.
///
/// The attributes come from `openehr_rm::v1_2::model`, looked up on the node's
/// class and on its concrete descendants, since a `DV_TEXT` node may hold a
/// `DV_CODED_TEXT` and a `PARTY_PROXY` node a `PARTY_IDENTIFIED`
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/data_types.html#_dv_coded_text_class>).
/// The class is the most general concrete one that declares the tail's first
/// attribute. A tail below a structural node other than a [`FAMILIES`] entry,
/// and a tail that steps through or ends on a container attribute, is `None`:
/// the engine walks a tail by attribute name and writes one value into the
/// node. No specification governs the walk: our own design.
#[must_use]
pub fn carried(rm_type: &str, tail: &[&str]) -> Option<(&'static str, Carried)> {
    let wanted = tail.join("/");
    if let Some(&(owner, _)) = FAMILIES
        .iter()
        .find(|&&(owner, path)| path == wanted && model::is_a(rm_type, owner))
    {
        return Some((owner, Carried::Family));
    }
    if derive::is_structural(rm_type) {
        return None;
    }
    let (&first, rest) = tail.split_first()?;
    let owner = owner_of(rm_type, first)?;
    let mut attribute = model::attribute(owner, first)?;
    for &segment in rest {
        if attribute.container != Container::None {
            return None;
        }
        attribute = concrete_forms(attribute.declared_type)
            .into_iter()
            .find_map(|form| model::attribute(form, segment))?;
    }
    if attribute.container != Container::None {
        return None;
    }
    Some((owner, held(attribute.declared_type)?))
}

/// Returns the most general concrete class at or below `rm_type` that
/// declares `attribute`, earliest in the model's order on a tie.
fn owner_of(rm_type: &str, attribute: &str) -> Option<&'static str> {
    let declaring: Vec<&'static str> = concrete_forms(rm_type)
        .into_iter()
        .filter(|form| model::class(form).is_some_and(|class| !class.is_abstract))
        .filter(|form| model::attribute(form, attribute).is_some())
        .collect();
    declaring.iter().copied().find(|&form| {
        !declaring
            .iter()
            .any(|&other| other != form && model::is_a(form, other))
    })
}

/// Returns the reference-model class plus every concrete class below it.
fn concrete_forms(rm_type: &str) -> Vec<&'static str> {
    let Some(class) = model::class(rm_type) else {
        return Vec::new();
    };
    let mut forms = vec![class.name];
    for &descendant in class.descendants {
        if !forms.contains(&descendant) {
            forms.push(descendant);
        }
    }
    forms
}

/// Returns what an attribute of `declared_type` holds, `None` for a type the
/// engine writes no scalar or value of.
///
/// The primitive names are the BASE foundation types the RM BMM declares
/// (<https://specifications.openehr.org/releases/BASE/Release-1.2.0/foundation_types.html>).
fn held(declared_type: &str) -> Option<Carried> {
    match declared_type {
        "String" => Some(Carried::Text),
        "Real" => Some(Carried::Real),
        "Integer" | "Integer64" => Some(Carried::Integer),
        "Boolean" => Some(Carried::Boolean),
        other => model::class(other).map(|_| Carried::Value),
    }
}

// TODO(#241): the `_provider` family is spelled here and in `family::provider`
// until openehr-sdt honours `|raw` on `_`-prefixed attribute families (sibling
// request S1).
/// The attributes of a structural class FLAT writes as a `_`-prefixed family
/// of the class's node.
///
/// `ENTRY.provider` is a `PARTY_PROXY`
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html#_entry_class>),
/// and Simplified Formats spells it `_provider` under the entry's node
/// (`docs/specs/its-rest/docs/simplified_formats/master05-rm_mapping.adoc`,
/// §OBSERVATION), the shape `family` spells `_other_participation` in.
pub const FAMILIES: &[(&str, &str)] = &[("ENTRY", "provider")];

/// Returns the FLAT family a tail of [`Carried::Family`] is written under.
#[must_use]
pub fn family_of(tail: &[&str]) -> Vec<Segment> {
    tail.iter()
        .map(|segment| Segment {
            name: format!("_{segment}"),
            index: None,
        })
        .collect()
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
    use super::Carried;
    use super::RmValue;
    use super::carried;
    use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
    use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;

    #[test]
    fn a_code_phrase_writes_all_three_attributes_in_its_canonical_form() {
        let value = RmValue::CodePhrase(CodePhrase {
            terminology_id: TerminologyId {
                value: String::from("local"),
            },
            code_string: String::from("at0017"),
            preferred_term: Some(String::from("Working")),
        });
        let canonical = value.to_canonical().expect("a code phrase encodes");
        assert_eq!(
            canonical,
            serde_json::json!({
                "_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                "code_string": "at0017",
                "preferred_term": "Working"
            })
        );
        assert_eq!(
            RmValue::from_canonical("CODE_PHRASE", "/x", &canonical).expect("it reads back"),
            value
        );
    }

    #[test]
    fn a_date_time_keeps_its_text_byte_for_byte() {
        let value = RmValue::from_canonical(
            "DV_DATE_TIME",
            "/context/start_time",
            &serde_json::json!({"_type": "DV_DATE_TIME", "value": "2026-09-12T10:00:00+02:00"}),
        )
        .expect("the canonical form reads back");
        let canonical = value.to_canonical().expect("a date and time encodes");
        assert_eq!(
            canonical.get("value").and_then(serde_json::Value::as_str),
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
    fn a_coded_text_keeps_its_term_mappings_and_language() {
        let written = serde_json::json!({
            "_type": "DV_CODED_TEXT",
            "value": "Synthetic problem one",
            "mappings": [{
                "_type": "TERM_MAPPING",
                "match": "=",
                "target": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "SNOMED-CT"},
                    "code_string": "73211009"
                }
            }],
            "language": {
                "_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_639-1"},
                "code_string": "en"
            },
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "local"},
                "code_string": "at0001"
            }
        });
        let value = RmValue::from_canonical("DV_CODED_TEXT", "/x", &written)
            .expect("the canonical form reads back");
        let canonical = value.to_canonical().expect("a coded text encodes");
        assert_eq!(canonical.get("mappings"), written.get("mappings"));
        assert_eq!(canonical.get("language"), written.get("language"));
    }

    #[test]
    fn a_defective_value_names_the_json_path_it_failed_at() {
        let error = RmValue::from_canonical(
            "DV_TEXT",
            "/x",
            &serde_json::json!({"_type": "DV_TEXT", "value": "text", "undeclared": 1}),
        )
        .expect_err("the strict reader refuses an undeclared key");
        assert!(
            core::error::Error::source(&error).is_some(),
            "the reader's refusal is the source: {error}"
        );
    }

    #[test]
    fn a_tail_reaches_every_attribute_the_model_gives_the_class() {
        assert_eq!(
            carried("DV_TEXT", &["hyperlink", "value"]),
            Some(("DV_TEXT", Carried::Text))
        );
        assert_eq!(
            carried("DV_TEXT", &["language"]),
            Some(("DV_TEXT", Carried::Value))
        );
        assert_eq!(
            carried("DV_DATE_TIME", &["normal_status", "code_string"]),
            Some(("DV_DATE_TIME", Carried::Text))
        );
        assert_eq!(
            carried("DV_PROPORTION", &["numerator"]),
            Some(("DV_PROPORTION", Carried::Real))
        );
        assert_eq!(
            carried("DV_INTERVAL", &["lower_included"]),
            Some(("DV_INTERVAL", Carried::Boolean))
        );
    }

    #[test]
    fn a_tail_only_a_subtype_declares_writes_the_subtype() {
        assert_eq!(
            carried("DV_TEXT", &["defining_code", "code_string"]),
            Some(("DV_CODED_TEXT", Carried::Text))
        );
        assert_eq!(
            carried("PARTY_PROXY", &["name"]),
            Some(("PARTY_IDENTIFIED", Carried::Text))
        );
    }

    #[test]
    fn a_container_tail_and_a_structural_tail_are_not_carried() {
        assert_eq!(carried("DV_TEXT", &["mappings", "match"]), None);
        assert_eq!(carried("DV_TEXT", &["mappings"]), None);
        assert_eq!(carried("EVALUATION", &["other_participations"]), None);
        assert_eq!(carried("DV_TEXT", &["no_such_attribute"]), None);
    }

    #[test]
    fn the_entry_provider_is_a_family() {
        assert_eq!(
            carried("EVALUATION", &["provider"]),
            Some(("ENTRY", Carried::Family))
        );
    }
}
