// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HL7 v2 logical models as the v2ig source of truth spells them.
//!
//! The files are FHIR `StructureDefinition`s of kind `logical`
//! (<https://hl7.org/fhir/R5/structuredefinition.html>), so the names follow
//! the FHIR JSON representation. The projection is its own and not
//! [`crate::fhir::StructureDefinition`]: the v2 files write some elements
//! with a JSON type FHIR does not allow (`max` as a number, `valueInteger` as a
//! string), and widening the shared projection would let the FHIR packages
//! carry the same defects unnoticed. Every object here refuses a member it
//! does not name, so a shape the generator has not been taught is loud; a
//! member the generator does not read is named and discarded.

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::fhir::{Derivation, StructureKind};

/// A v2 `StructureDefinition`: a segment, a message structure, or a base.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructureDefinition {
    /// Always `StructureDefinition`.
    pub resource_type: String,
    /// The canonical URL, for example `http://hl7.org/v2/StructureDefinition/OBX`.
    pub url: String,
    /// The resource id, for example `OBX` or `ORU_R01-A`.
    pub id: String,
    /// The computer-friendly name.
    pub name: String,
    /// The human title.
    pub title: Option<String>,
    /// Always `logical` for the v2 models.
    pub kind: StructureKind,
    /// Whether the model is an abstract base.
    #[serde(rename = "abstract")]
    pub is_abstract: bool,
    /// The type the model defines.
    #[serde(rename = "type")]
    pub type_name: String,
    /// The base the model specializes.
    pub base_definition: Option<String>,
    /// `specialization` for every segment and message structure.
    pub derivation: Option<Derivation>,
    /// The FHIR version the file declares.
    pub fhir_version: Option<String>,
    /// A snapshot, which no v2 model ships; a present one is refused at lowering.
    pub snapshot: Option<serde_json::Value>,
    /// The elements, complete over an element-less base.
    pub differential: Option<Differential>,
    /// Not read.
    pub meta: Option<IgnoredAny>,
    /// Not read.
    pub version: Option<IgnoredAny>,
    /// Not read.
    pub status: Option<IgnoredAny>,
    /// Not read.
    pub date: Option<IgnoredAny>,
    /// Not read.
    pub description: Option<IgnoredAny>,
    /// Not read: the standards status, normative version and work group of
    /// a primitive data type.
    pub extension: Option<IgnoredAny>,
}

/// `StructureDefinition.differential`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Differential {
    /// The elements in definition order.
    pub element: Vec<Element>,
}

/// A number or a string where FHIR allows only one of them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    /// A JSON number without a fraction.
    Integer(u64),
    /// A JSON string.
    Text(String),
}

/// One `ElementDefinition` of a v2 model.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Element {
    /// The position-prefixed element id, for example `OBX.1-setId`.
    pub id: String,
    /// The element path, equal to the id in every v2 model.
    pub path: String,
    /// The element name as HL7 writes it, for example `Set ID – OBX`.
    pub short: Option<String>,
    /// The minimum cardinality.
    pub min: Option<u32>,
    /// The maximum cardinality: a number or `*`, as text or as a JSON number.
    pub max: Option<Scalar>,
    /// The element type: a v2 data type code, a canonical segment URL, or
    /// `BackboneElement` for a segment group.
    #[serde(rename = "type")]
    pub types: Option<Vec<ElementType>>,
    /// A `#id` reference to the element whose content this one repeats.
    pub content_reference: Option<String>,
    /// The table binding.
    pub binding: Option<Binding>,
    /// The v2 facts carried as extensions.
    #[serde(default)]
    pub extension: Vec<Extension>,
    // NOTE: the audit of #251 found `code[].code` a JSON number and `definition`
    // null in segment/segments/*.json; neither is read, so neither defect is typed.
    /// Not read.
    pub code: Option<IgnoredAny>,
    /// Not read.
    pub definition: Option<IgnoredAny>,
    /// The member `definition` misspelled, which the complex data type files
    /// write; not read, and a defect outside the files that carry it.
    pub defintion: Option<IgnoredAny>,
    /// Not read.
    pub comment: Option<IgnoredAny>,
}

/// One entry of `ElementDefinition.type`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementType {
    /// The type code.
    pub code: String,
}

/// `ElementDefinition.binding`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    /// The binding strength.
    pub strength: String,
    /// The bound value set's canonical URL.
    pub value_set: String,
}

/// An extension on an element, possibly with nested extensions.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Extension {
    /// The extension URL: absolute at the top level, a bare name when nested.
    pub url: String,
    /// The value when the extension carries a `code`.
    pub value_code: Option<String>,
    /// The value when the extension carries an `integer`, whatever JSON type it is written in.
    pub value_integer: Option<Scalar>,
    /// The value when the extension carries a `boolean`.
    pub value_boolean: Option<bool>,
    /// The nested extensions of a complex extension.
    #[serde(default)]
    #[expect(
        clippy::struct_field_names,
        reason = "FHIR names the nested member of an extension `extension`"
    )]
    pub extension: Vec<Extension>,
}

/// A v2 message definition: a constraint on the `Message` base that names
/// the message code, the trigger event and the structure.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageDefinition {
    /// Always `StructureDefinition`.
    pub resource_type: String,
    /// The canonical URL, for example `http://hl7.org/v2/StructureDefinition/Message/ORU-R01`.
    pub url: String,
    /// The resource id, for example `ORU-R01`.
    pub id: String,
    /// Always `logical` for the v2 models.
    pub kind: StructureKind,
    /// Whether the model is an abstract base.
    #[serde(rename = "abstract")]
    pub is_abstract: bool,
    /// The type the model constrains, `Message`.
    #[serde(rename = "type")]
    pub type_name: String,
    /// The base the definition constrains.
    pub base_definition: Option<String>,
    /// `constraint` for every message definition.
    pub derivation: Option<Derivation>,
    /// The constrained elements.
    pub differential: Option<MessageDifferential>,
    /// Not read.
    pub name: Option<IgnoredAny>,
    /// Not read.
    pub title: Option<IgnoredAny>,
    /// Not read.
    pub version: Option<IgnoredAny>,
    /// Not read.
    pub status: Option<IgnoredAny>,
    /// Not read.
    pub date: Option<IgnoredAny>,
    /// Not read.
    pub description: Option<IgnoredAny>,
}

/// The differential of a [`MessageDefinition`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDifferential {
    /// The elements in definition order.
    pub element: Vec<MessageElement>,
}

/// One constrained element of a [`MessageDefinition`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageElement {
    /// The element id, for example `Message.triggerEvent`.
    pub id: String,
    /// The element path, equal to the id in every message definition.
    pub path: String,
    /// The fixed code of `messageType`, `triggerEvent` and `status`.
    pub pattern_code: Option<String>,
    /// The reference type of `structure` and the acknowledgment elements.
    #[serde(rename = "type")]
    pub types: Option<Vec<ReferenceType>>,
}

/// One entry of a [`MessageElement`]'s `type`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReferenceType {
    /// The type code, `Reference`.
    pub code: String,
    /// The profiles the reference targets.
    pub target_profile: Vec<String>,
}
