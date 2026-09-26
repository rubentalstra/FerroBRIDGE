// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR elements the data-type cells read and write.
//!
//! [`crate::tree`] reads and writes the lexical `Value` of `fhir-types`, and a
//! [`crate::engine::lens::Lens`] is typed in the generated element struct, so
//! this module is the one place the two meet. Nothing here models a FHIR
//! element: the generated `Json` and `Primitive` codecs do the work, and
//! [`FhirValue`] only says which of them to run.

use fhir_types::codec::DecodeError;
use fhir_types::codec::EncodeError;
use fhir_types::codec::Json;
use fhir_types::codec::Path;
use fhir_types::codec::Primitive;
use fhir_types::codec::Value;
use fhir_types::r4::codeable_concept::CodeableConcept;
use fhir_types::r4::coding::Coding;
use fhir_types::r4::identifier::Identifier;
use fhir_types::r4::period::Period;
use fhir_types::r4::primitives::DateTime;
use fhir_types::r4::primitives::String as FhirString;
use fhir_types::r4::quantity::Quantity;
use fhir_types::r4::reference::Reference;
use fhir_types::schema::ValueKind;

use crate::model::ast::keyword::DataType;
use crate::tree::element::Location;

/// Which FHIR element a mapping writes.
///
/// The `type` key of a `with` block names it where a mapping carries one
/// (`types-of-mappings/data-type/data-mappings.adoc`); otherwise the element
/// table decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FhirKind {
    /// A `string`, and every other primitive that travels as text.
    String,
    /// A `dateTime`.
    DateTime,
    /// A `Coding`.
    Coding,
    /// A `CodeableConcept`.
    CodeableConcept,
    /// A `Period`.
    Period,
    /// A `Reference`.
    Reference,
    /// An `Identifier`.
    Identifier,
    /// A `Quantity`.
    Quantity,
}

impl FhirKind {
    /// Returns the kind the `type` key names, `None` for `NONE` and for the
    /// kinds no cell of this milestone carries.
    #[must_use]
    pub const fn of(data_type: DataType) -> Option<Self> {
        match data_type {
            DataType::String | DataType::Id => Some(Self::String),
            DataType::DateTime => Some(Self::DateTime),
            DataType::Coding => Some(Self::Coding),
            DataType::CodeableConcept => Some(Self::CodeableConcept),
            DataType::Identifier => Some(Self::Identifier),
            DataType::Quantity | DataType::Proportion => Some(Self::Quantity),
            DataType::None | DataType::Dosage => None,
        }
    }

    /// Returns the kind a FHIR type code names, `None` for a type no cell
    /// carries.
    ///
    /// The codes are the `ElementDefinition.type.code` values of the element
    /// table (<https://hl7.org/fhir/R4/elementdefinition.html>), as
    /// [`crate::resolve::derive`] derives them for a mapping with no `type`
    /// key.
    #[must_use]
    pub fn of_code(code: &str) -> Option<Self> {
        match code {
            "string" => Some(Self::String),
            "dateTime" => Some(Self::DateTime),
            "Coding" => Some(Self::Coding),
            "CodeableConcept" => Some(Self::CodeableConcept),
            "Period" => Some(Self::Period),
            "Reference" => Some(Self::Reference),
            "Identifier" => Some(Self::Identifier),
            "Quantity" => Some(Self::Quantity),
            _ => None,
        }
    }

    /// Returns the kind the element table resolved a path to.
    ///
    /// A primitive element travels as text unless the mapping's `type` key
    /// says otherwise, so only the complex types the cells carry are named
    /// here.
    #[must_use]
    pub fn at(location: &Location) -> Option<Self> {
        match *location {
            Location::Complex(schema) => match schema.path {
                "Coding" => Some(Self::Coding),
                "CodeableConcept" => Some(Self::CodeableConcept),
                "Period" => Some(Self::Period),
                "Reference" => Some(Self::Reference),
                "Identifier" => Some(Self::Identifier),
                "Quantity" => Some(Self::Quantity),
                _ => None,
            },
            Location::Primitive(ValueKind::Text) | Location::Attribute => Some(Self::String),
            Location::Primitive(_)
            | Location::PrimitiveElement
            | Location::Choice(_)
            | Location::Resource
            | Location::Deferred => None,
        }
    }

    /// Returns the name the diagnostics use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::DateTime => "dateTime",
            Self::Coding => "Coding",
            Self::CodeableConcept => "CodeableConcept",
            Self::Period => "Period",
            Self::Reference => "Reference",
            Self::Identifier => "Identifier",
            Self::Quantity => "Quantity",
        }
    }
}

/// One FHIR element a data-type cell produced or consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FhirValue {
    /// A `string`.
    String(FhirString),
    /// A `dateTime`.
    DateTime(DateTime),
    /// A `Coding`.
    Coding(Box<Coding>),
    /// A `CodeableConcept`.
    CodeableConcept(Box<CodeableConcept>),
    /// A `Period`.
    Period(Box<Period>),
    /// A `Reference`.
    Reference(Box<Reference>),
    /// An `Identifier`.
    Identifier(Box<Identifier>),
    /// A `Quantity`.
    Quantity(Box<Quantity>),
}

/// Why a FHIR element could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FhirError {
    /// The document does not decode as the element the table names.
    #[error("the value at {element} does not read as {kind}")]
    Decode {
        /// The element path from the table.
        element: String,
        /// The kind the mapping asked for.
        kind: &'static str,
        /// The refusal the generated codec returned.
        #[source]
        source: DecodeError,
    },
    /// The element has no JSON form.
    #[error("the {kind} value has no JSON form")]
    Encode {
        /// The kind that refused.
        kind: &'static str,
        /// The refusal the generated codec returned.
        #[source]
        source: EncodeError,
    },
    /// A primitive carried only an `id` or an `extension`.
    #[error("the {kind} value at {element} carries no value of its own")]
    NoValue {
        /// The element path from the table.
        element: String,
        /// The kind the mapping asked for.
        kind: &'static str,
    },
}

impl FhirValue {
    /// Returns the kind of the element.
    #[must_use]
    pub const fn kind(&self) -> FhirKind {
        match *self {
            Self::String(_) => FhirKind::String,
            Self::DateTime(_) => FhirKind::DateTime,
            Self::Coding(_) => FhirKind::Coding,
            Self::CodeableConcept(_) => FhirKind::CodeableConcept,
            Self::Period(_) => FhirKind::Period,
            Self::Reference(_) => FhirKind::Reference,
            Self::Identifier(_) => FhirKind::Identifier,
            Self::Quantity(_) => FhirKind::Quantity,
        }
    }

    /// Reads the value a document holds at `element` as `kind`.
    ///
    /// # Errors
    ///
    /// Returns [`FhirError::Decode`] when the document does not decode as the
    /// element the mapping asked for.
    pub fn read(kind: FhirKind, element: &str, value: &Value) -> Result<Self, FhirError> {
        let mut path = Path::root(element);
        let refuse = |source: DecodeError| FhirError::Decode {
            element: String::from(element),
            kind: kind.as_str(),
            source,
        };
        match kind {
            FhirKind::String => FhirString::from_json_parts(Some(value), None, &mut path)
                .map(Self::String)
                .map_err(refuse),
            FhirKind::DateTime => DateTime::from_json_parts(Some(value), None, &mut path)
                .map(Self::DateTime)
                .map_err(refuse),
            FhirKind::Coding => object(value, element, kind)
                .and_then(|object| Coding::from_json(object, &mut path).map_err(refuse))
                .map(|coding| Self::Coding(Box::new(coding))),
            FhirKind::CodeableConcept => object(value, element, kind)
                .and_then(|object| CodeableConcept::from_json(object, &mut path).map_err(refuse))
                .map(|concept| Self::CodeableConcept(Box::new(concept))),
            FhirKind::Period => object(value, element, kind)
                .and_then(|object| Period::from_json(object, &mut path).map_err(refuse))
                .map(|period| Self::Period(Box::new(period))),
            FhirKind::Reference => object(value, element, kind)
                .and_then(|object| Reference::from_json(object, &mut path).map_err(refuse))
                .map(|reference| Self::Reference(Box::new(reference))),
            FhirKind::Identifier => object(value, element, kind)
                .and_then(|object| Identifier::from_json(object, &mut path).map_err(refuse))
                .map(|identifier| Self::Identifier(Box::new(identifier))),
            FhirKind::Quantity => object(value, element, kind)
                .and_then(|object| Quantity::from_json(object, &mut path).map_err(refuse))
                .map(|quantity| Self::Quantity(Box::new(quantity))),
        }
    }

    /// Returns the JSON the element writes into a document.
    ///
    /// # Errors
    ///
    /// Returns [`FhirError::Encode`] when the generated codec refuses the
    /// value and [`FhirError::NoValue`] for a primitive that carries no value
    /// of its own.
    pub fn write(&self, element: &str) -> Result<Value, FhirError> {
        let kind = self.kind().as_str();
        let complex = |result: Result<fhir_types::codec::Object, EncodeError>| {
            result
                .map(Value::Object)
                .map_err(|source| FhirError::Encode { kind, source })
        };
        let primitive = |result: Result<Option<Value>, EncodeError>| {
            result
                .map_err(|source| FhirError::Encode { kind, source })?
                .ok_or_else(|| FhirError::NoValue {
                    element: String::from(element),
                    kind,
                })
        };
        match *self {
            Self::String(ref text) => primitive(text.value_json()),
            Self::DateTime(ref date) => primitive(date.value_json()),
            Self::Coding(ref coding) => complex(coding.to_json()),
            Self::CodeableConcept(ref concept) => complex(concept.to_json()),
            Self::Period(ref period) => complex(period.to_json()),
            Self::Reference(ref reference) => complex(reference.to_json()),
            Self::Identifier(ref identifier) => complex(identifier.to_json()),
            Self::Quantity(ref quantity) => complex(quantity.to_json()),
        }
    }
}

/// Returns the object a complex element is written as.
fn object<'tree>(
    value: &'tree Value,
    element: &str,
    kind: FhirKind,
) -> Result<&'tree fhir_types::codec::Object, FhirError> {
    let path = Path::root(element);
    fhir_types::codec::expect_object(value, &path).map_err(|source| FhirError::Decode {
        element: String::from(element),
        kind: kind.as_str(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::FhirKind;
    use super::FhirValue;
    use fhir_types::codec::Value;

    #[test]
    fn a_coding_reads_and_writes_through_the_generated_codec() {
        let document = Value::from_serde_json(serde_json::json!({
            "system": "http://example.org/fhir/CodeSystem/synthetic-problems",
            "code": "SYN-001"
        }));
        let value = FhirValue::read(FhirKind::Coding, "Condition.code.coding", &document)
            .expect("the object reads as a Coding");
        assert_eq!(value.kind(), FhirKind::Coding);
        let written = value
            .write("Condition.code.coding")
            .expect("a Coding has a JSON form");
        assert_eq!(written, document, "the codec round trips the element");
    }

    #[test]
    fn a_date_time_keeps_the_text_the_document_carried() {
        let document = Value::String(String::from("2026-09-13T10:00:00.123456+02:00"));
        let value = FhirValue::read(FhirKind::DateTime, "Condition.recordedDate", &document)
            .expect("the string reads as a dateTime");
        assert_eq!(
            value
                .write("Condition.recordedDate")
                .expect("a dateTime has a JSON form"),
            document,
            "the fractional seconds and the offset survive"
        );
    }

    #[test]
    fn every_pair_the_resolver_derives_names_a_kind_the_cells_carry() {
        for &(class, codes) in crate::resolve::derive::PAIRS {
            for code in codes {
                assert!(
                    FhirKind::of_code(code).is_some(),
                    "{class} pairs with {code}, which no kind carries"
                );
            }
        }
        assert_eq!(
            FhirKind::of_code(crate::resolve::derive::TEXT),
            Some(FhirKind::String)
        );
    }

    #[test]
    fn a_value_of_the_wrong_shape_names_the_element() {
        let document = Value::String(String::from("not an object"));
        let error = FhirValue::read(FhirKind::Coding, "Condition.code.coding", &document)
            .expect_err("a string is no Coding");
        assert!(
            error.to_string().contains("Condition.code.coding"),
            "the refusal names the element: {error}"
        );
    }
}
