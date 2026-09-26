// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data-type pair a mapping with no `type` key converts through.
//!
//! The `type` key is deprecated "since the information is derivable from the
//! instances of FHIR and openEHR"
//! (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/data-mappings.adoc`,
//! §Deprecated), so the compiler derives the pair once, from the two sides it
//! already resolved: the FHIR type code the element table gives the element
//! and the reference-model class the Web Template gives the node. The pairs
//! themselves are the rows of the data-type chapter
//! (`types-of-mappings/data-type/*.adoc`) that the engine's cells carry.
//!
//! The chapter fixes which pairs convert and no order among them, so which
//! pair wins when a side admits several is our own design: the order a class's
//! chapter page lists its pairs in.

use fhir_types::schema::Kind;
use fhir_types::schema::ValueKind;
use openehr_rm::v1_2::model;

use crate::model::ast::keyword::DataType;

/// The FHIR type codes each openEHR class converts with, in the order the
/// class's page of the data-type chapter lists them.
///
/// A row holds only the pairs a data-type cell of the engine carries both
/// ways, so a derived pair is always one the engine runs. `DV_INTERVAL`
/// against `Period` is the engine's `DV_INTERVAL<DV_DATE_TIME>` cell.
pub const PAIRS: &[(&str, &[&str])] = &[
    ("DV_TEXT", &["string", "Coding", "CodeableConcept"]),
    ("DV_CODED_TEXT", &["CodeableConcept", "Coding"]),
    ("CODE_PHRASE", &["Coding"]),
    ("DV_DATE_TIME", &["dateTime", "Period"]),
    ("DV_INTERVAL", &["Period"]),
    ("PARTY_IDENTIFIED", &["Reference"]),
    ("PARTY_PROXY", &["Reference"]),
    ("DV_IDENTIFIER", &["Identifier"]),
    ("DV_PROPORTION", &["Quantity"]),
];

/// The FHIR type code every primitive that travels as a JSON string falls
/// back to.
pub const TEXT: &str = "string";

/// The reference-model class every node that holds other nodes descends from.
///
/// `PATHABLE` is the root of the classes a path can step through
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_pathable_class>),
/// and no data value is one.
const STRUCTURAL: &str = "PATHABLE";

/// Returns the FHIR type codes `class` converts with, empty for a class no
/// cell carries.
#[must_use]
pub fn pairs(class: &str) -> &'static [&'static str] {
    PAIRS
        .iter()
        .find(|&&(name, _)| name == class)
        .map_or(&[], |&(_, codes)| codes)
}

/// Returns whether `class` is a structural node, which holds other nodes and
/// carries no data value of its own.
#[must_use]
pub fn is_structural(class: &str) -> bool {
    model::is_a(class, STRUCTURAL)
}

/// Returns the FHIR type code an element of type `code` converts through
/// against a node of `class`.
///
/// The element's own type wins when the class pairs with it. A primitive that
/// travels as text and is no pair of the class converts as a `string`, the
/// FHIR type every such primitive specializes in its value space
/// (<https://hl7.org/fhir/R4/datatypes.html#primitive>), which is what the
/// engine did before the pair was derived.
#[must_use]
pub fn element(class: &str, code: &'static str, text: bool) -> &'static str {
    if text && !pairs(class).contains(&code) {
        return TEXT;
    }
    code
}

/// Returns the FHIR type code of the alternative a document carries for a
/// choice element, against a node of `class`.
///
/// A complex alternative names its type. A primitive's suffix is its type
/// code with the first letter raised (<https://hl7.org/fhir/R4/formats.html#choice>),
/// so it is matched against the class's pairs without regard to case, and an
/// alternative that travels as text and is no pair converts as a `string`.
/// `None` for a primitive that travels as a JSON boolean or number, which no
/// cell carries.
#[must_use]
pub fn carried(class: &str, suffix: &str, kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Complex(name) => Some(name),
        Kind::Primitive(ValueKind::Text) | Kind::Attribute | Kind::Xhtml => Some(
            pairs(class)
                .iter()
                .find(|code| code.eq_ignore_ascii_case(suffix))
                .copied()
                .unwrap_or(TEXT),
        ),
        Kind::Primitive(ValueKind::Boolean | ValueKind::Integer | ValueKind::Decimal)
        | Kind::Resource
        | Kind::Choice(_) => None,
    }
}

/// Returns the FHIR type code the `type` key names, `None` for `NONE`.
///
/// The key's identifiers are the Type IDs of the deprecated table in
/// `types-of-mappings/data-type/data-mappings.adoc`; `PROPORTION` travels as
/// the `Quantity` the engine's `DV_PROPORTION` cell carries.
#[must_use]
pub const fn declared(data_type: DataType) -> Option<&'static str> {
    match data_type {
        DataType::Quantity | DataType::Proportion => Some("Quantity"),
        DataType::DateTime => Some("dateTime"),
        DataType::CodeableConcept => Some("CodeableConcept"),
        DataType::Coding => Some("Coding"),
        DataType::String => Some("string"),
        DataType::Dosage => Some("Dosage"),
        DataType::Id => Some("id"),
        DataType::Identifier => Some("Identifier"),
        DataType::None => None,
    }
}

/// Returns the suffix of the alternative of a choice element that has the
/// type `code`, `None` when the choice admits no such alternative.
#[must_use]
pub fn suffix_of(code: &str, variants: &[(&'static str, Kind)]) -> Option<&'static str> {
    variants
        .iter()
        .find(|&&(suffix, _)| suffix.eq_ignore_ascii_case(code))
        .map(|&(suffix, _)| suffix)
}

/// Returns the suffix of the alternative a choice element is written as,
/// against a node of `class`.
///
/// The first pair of the class the choice admits is the one written; `None`
/// when the choice admits none of them.
#[must_use]
pub fn alternative(class: &str, variants: &[(&'static str, Kind)]) -> Option<&'static str> {
    pairs(class)
        .iter()
        .find_map(|code| suffix_of(code, variants))
}

#[cfg(test)]
mod tests {
    use fhir_types::schema::Kind;
    use fhir_types::schema::ValueKind;

    use super::alternative;
    use super::carried;
    use super::element;
    use super::is_structural;

    #[test]
    fn a_date_time_element_against_a_date_time_node_converts_as_a_date_time() {
        assert_eq!(element("DV_DATE_TIME", "dateTime", true), "dateTime");
    }

    #[test]
    fn a_date_time_element_against_a_text_node_converts_as_text() {
        assert_eq!(element("DV_TEXT", "dateTime", true), "string");
    }

    #[test]
    fn a_complex_element_keeps_its_type_whatever_the_class() {
        assert_eq!(element("DV_TEXT", "Annotation", false), "Annotation");
    }

    #[test]
    fn the_structural_classes_are_the_pathable_ones() {
        for class in [
            "EVALUATION",
            "CLUSTER",
            "COMPOSITION",
            "SECTION",
            "EVENT_CONTEXT",
        ] {
            assert!(is_structural(class), "{class} holds other nodes");
        }
        for class in [
            "DV_CODED_TEXT",
            "CODE_PHRASE",
            "PARTY_PROXY",
            "DV_DATE_TIME",
        ] {
            assert!(!is_structural(class), "{class} is a value");
        }
    }

    /// The alternatives `Extension.value[x]` admits that the pairs name.
    const VALUES: &[(&str, Kind)] = &[
        ("CodeableConcept", Kind::Complex("CodeableConcept")),
        ("Coding", Kind::Complex("Coding")),
        ("DateTime", Kind::Primitive(ValueKind::Text)),
        ("Period", Kind::Complex("Period")),
        ("String", Kind::Primitive(ValueKind::Text)),
    ];

    #[test]
    fn a_choice_is_written_as_the_first_pair_it_admits() {
        assert_eq!(
            alternative("DV_CODED_TEXT", VALUES),
            Some("CodeableConcept")
        );
        assert_eq!(alternative("CODE_PHRASE", VALUES), Some("Coding"));
        assert_eq!(alternative("DV_TEXT", VALUES), Some("String"));
        assert_eq!(alternative("DV_DATE_TIME", VALUES), Some("DateTime"));
    }

    #[test]
    fn a_choice_admitting_no_pair_of_the_class_has_no_alternative() {
        assert_eq!(alternative("DV_IDENTIFIER", VALUES), None);
        assert_eq!(alternative("DV_BOOLEAN", VALUES), None);
    }

    #[test]
    fn the_alternative_a_document_carries_names_its_type_code() {
        assert_eq!(
            carried("DV_DATE_TIME", "DateTime", Kind::Primitive(ValueKind::Text)),
            Some("dateTime")
        );
        assert_eq!(
            carried("DV_CODED_TEXT", "Coding", Kind::Complex("Coding")),
            Some("Coding")
        );
        assert_eq!(
            carried("DV_CODED_TEXT", "Code", Kind::Primitive(ValueKind::Text)),
            Some("string")
        );
        assert_eq!(
            carried("DV_TEXT", "Boolean", Kind::Primitive(ValueKind::Boolean)),
            None
        );
    }
}
