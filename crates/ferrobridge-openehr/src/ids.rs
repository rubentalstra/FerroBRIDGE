// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identifiers the CDR wire carries.
//!
//! The version, version container and template identifiers are the
//! `openehr-base` BASE 1.3 types the ITS-REST DTOs use; this module holds the
//! thin readers the wire needs over them (an `ETag` value, the version
//! container of a version) and the handles openEHR has no type of their own
//! for.

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::template_id::TemplateId;
use std::fmt;

/// A character that would change the shape of a request path or query.
const STRUCTURAL: [char; 3] = ['/', '?', '#'];

/// An identifier could not be built from the given text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdError {
    /// The text was empty.
    #[error("a {kind} must not be empty")]
    Empty {
        /// The identifier kind that was being built.
        kind: &'static str,
    },
    /// The text carried a character that would change the request path.
    #[error("a {kind} must not contain {character:?}")]
    ForbiddenCharacter {
        /// The identifier kind that was being built.
        kind: &'static str,
        /// The offending character.
        character: char,
    },
    /// The text is not the openEHR identifier its position requires.
    #[error("{found:?} is not a legal {kind}")]
    Openehr {
        /// The identifier kind that was being built.
        kind: &'static str,
        /// The text that was parsed.
        found: String,
        /// What the BASE 1.3 identifier grammar refused.
        #[source]
        source: openehr_base::v1_3::base_types::identification::lexical::IdError,
    },
}

/// Returns `text` as an owned opaque identifier of `kind`, or the reason it is
/// not one.
fn opaque(kind: &'static str, text: &str) -> Result<String, IdError> {
    if text.is_empty() {
        return Err(IdError::Empty { kind });
    }
    if let Some(character) = text
        .chars()
        .find(|c| STRUCTURAL.contains(c) || c.is_control())
    {
        return Err(IdError::ForbiddenCharacter { kind, character });
    }
    Ok(text.to_owned())
}

/// Returns the entity-tag body of an `ETag` or `If-Match` field value.
///
/// ITS-REST 1.1.0 §Requests and responses/HTTP headers/ETag and Last-Modified
/// makes the `ETag` weak, so a served value carries the `W/` weakness
/// indicator and double quotes around the identifier; neither belongs to the
/// identifier itself.
#[must_use]
pub fn entity_tag(value: &str) -> &str {
    let value = value.strip_prefix("W/").unwrap_or(value);
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

/// An EHR identifier, taken from `EHR.ehr_id.value`.
///
/// The ITS-REST 1.1.0 EHR API declares the path parameter as a string of
/// `format: uuid` and its prose only recommends a UUID
/// (`ehr-codegen.openapi.yaml`, `components.parameters.ehr_id`, and the
/// `ehr_create_with_id` description: "It is strongly RECOMMENDED that an UUID
/// always be used for this"), so the value stays opaque here.
// NOTE: RM EHR §`ehr_id` is a HIER_OBJECT_ID, kept its own type (C-NEWTYPE) so an
// EHR handle and a version container can never swap places in a call.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EhrId(String);

impl EhrId {
    /// Returns the EHR identifier `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is empty or carries a character that
    /// would change the request path.
    pub fn new(text: &str) -> Result<Self, IdError> {
        opaque("ehr_id", text).map(Self)
    }

    /// Returns the identifier as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EhrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Returns the version identifier an `ETag` field value names.
///
/// The `W/` weakness indicator and the double quotes ITS-REST 1.1.0
/// §Requests and responses/HTTP headers/ETag and Last-Modified puts around the
/// value are stripped before BASE 1.3 parses it.
///
/// # Errors
/// Returns [`IdError::Openehr`] when what the entity tag carries is not an
/// `OBJECT_VERSION_ID`.
pub fn version_from_etag(value: &str) -> Result<ObjectVersionId, IdError> {
    let tag = entity_tag(value);
    ObjectVersionId::new(tag).map_err(|source| IdError::Openehr {
        kind: "OBJECT_VERSION_ID",
        found: tag.to_owned(),
        source,
    })
}

/// Returns the version container `version` belongs to, its `object_id` part
/// verbatim.
///
/// RM Common §`OBJECT_VERSION_ID` gives the three-part form
/// `object_id::creating_system_id::version_tree_id`, of which the leading
/// `object_id` names the version container and is stable across updates. The
/// part is taken as written, since the typed `Uid` accessor lower-cases a UUID
/// and the CDR addresses the container by the spelling it issued.
///
/// # Panics
/// Never: `ObjectVersionId::new` refuses a value whose `object_id` part is
/// not a legal `uid`, which is all `HierObjectId::new` checks.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "ObjectVersionId::new checked the object_id part against the uid production HierObjectId::new checks"
)]
pub fn versioned_object_uid(version: &ObjectVersionId) -> HierObjectId {
    let object_id = version
        .value()
        .split_once("::")
        .map_or(version.value(), |(head, _)| head);
    HierObjectId::new(object_id)
        .expect("the object_id of a constructed OBJECT_VERSION_ID should be a legal uid")
}

/// Returns the template identifier `text` names.
///
/// BASE 1.3 leaves the `TEMPLATE_ID` lexical form open and the ITS-REST 1.1.0
/// Definition API examples range from `Vital Signs` to a full HRID
/// (`definition-codegen.openapi.yaml`, `components.parameters.template_id`),
/// so a space is legal and only path-structural characters are refused.
///
/// # Errors
/// Returns [`IdError`] when `text` is empty or carries a character that would
/// change the request path.
pub fn template_id(text: &str) -> Result<TemplateId, IdError> {
    opaque("template_id", text).map(|value| TemplateId { value })
}

/// A CONTRIBUTION identifier, taken from `CONTRIBUTION.uid.value`.
// NOTE: RM Common §`CONTRIBUTION.uid` is a HIER_OBJECT_ID, kept its own type
// (C-NEWTYPE) so a contribution and a version container can never swap places.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContributionUid(String);

impl ContributionUid {
    /// Returns the CONTRIBUTION identifier `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is empty or carries a character that
    /// would change the request path.
    pub fn new(text: &str) -> Result<Self, IdError> {
        opaque("contribution_uid", text).map(Self)
    }

    /// Returns the identifier as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContributionUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The subject identifier an EHR lookup matches against
/// `EHR_STATUS.subject.external_ref.id.value`.
// NOTE: `PARTY_REF.id` admits any OBJECT_ID subtype and ITS-REST 1.1.0 passes it
// as a bare query string, so no BASE 1.3 type holds the lookup value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubjectId(String);

impl SubjectId {
    /// Returns the subject identifier `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is empty or carries a control
    /// character.
    pub fn new(text: &str) -> Result<Self, IdError> {
        if text.is_empty() {
            return Err(IdError::Empty { kind: "subject_id" });
        }
        if let Some(character) = text.chars().find(|c| c.is_control()) {
            return Err(IdError::ForbiddenCharacter {
                kind: "subject_id",
                character,
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns the identifier as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SubjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The namespace a [`SubjectId`] is drawn from, matched against
/// `EHR_STATUS.subject.external_ref.namespace`.
// NOTE: `OBJECT_REF.namespace` is a bare String in BASE 1.3, so the lookup key is
// its own type only to keep it apart from the subject id it travels with.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubjectNamespace(String);

impl SubjectNamespace {
    /// Returns the subject namespace `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is empty or carries a control
    /// character.
    pub fn new(text: &str) -> Result<Self, IdError> {
        if text.is_empty() {
            return Err(IdError::Empty {
                kind: "subject_namespace",
            });
        }
        if let Some(character) = text.chars().find(|c| c.is_control()) {
            return Err(IdError::ForbiddenCharacter {
                kind: "subject_namespace",
                character,
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns the namespace as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SubjectNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A correlation identifier the client echoes into `X-Request-Id`.
///
/// No specification governs this: our own design. The value is restricted to
/// printable ASCII so a caller-supplied identifier cannot inject a header line
/// or a log line.
// NOTE: no specification governs this: our own design, a transport correlation
// id that is no openEHR identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(String);

impl RequestId {
    /// Returns the request identifier `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is empty or carries anything but
    /// printable ASCII.
    pub fn new(text: &str) -> Result<Self, IdError> {
        if text.is_empty() {
            return Err(IdError::Empty { kind: "request id" });
        }
        if let Some(character) = text.chars().find(|c| !c.is_ascii_graphic()) {
            return Err(IdError::ForbiddenCharacter {
                kind: "request id",
                character,
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns the identifier as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "test assertions")]

    use super::{
        EhrId, IdError, RequestId, entity_tag, template_id, version_from_etag, versioned_object_uid,
    };
    use openehr_base::v1_3::base_types::identification::lexical::IdError as BaseIdError;

    /// A synthetic version container.
    const CONTAINER: &str = "8849182c-82ad-4088-a07f-48ead4180515";

    #[test]
    fn an_ehr_id_refuses_a_path_separator() {
        assert_eq!(
            Err(IdError::ForbiddenCharacter {
                kind: "ehr_id",
                character: '/'
            }),
            EhrId::new("7d44b88c/composition")
        );
    }

    #[test]
    fn an_ehr_id_refuses_the_empty_string() {
        assert_eq!(Err(IdError::Empty { kind: "ehr_id" }), EhrId::new(""));
    }

    #[test]
    fn a_template_id_keeps_the_legacy_spaced_form() -> Result<(), IdError> {
        assert_eq!("Vital Signs", template_id("Vital Signs")?.value);
        Ok(())
    }

    #[test]
    fn a_template_id_refuses_a_path_separator() {
        assert!(template_id("Vital/Signs").is_err());
        assert!(template_id("").is_err());
    }

    #[test]
    fn a_version_id_splits_into_its_three_parts() -> Result<(), IdError> {
        let text = format!("{CONTAINER}::openEHRSys.example.com::2");
        let id = version_from_etag(&text)?;
        assert_eq!(CONTAINER, versioned_object_uid(&id).value());
        assert_eq!("openEHRSys.example.com", id.creating_system_id_str());
        assert_eq!("2", id.version_tree_id().value());
        assert_eq!(text, id.value());
        Ok(())
    }

    #[test]
    fn the_version_container_keeps_the_spelling_the_cdr_issued() -> Result<(), IdError> {
        let upper = "8849182C-82AD-4088-A07F-48EAD4180515";
        let id = version_from_etag(&format!("{upper}::ferroehr::1"))?;
        assert_eq!(upper, versioned_object_uid(&id).value());
        Ok(())
    }

    #[test]
    fn a_version_id_refuses_a_two_part_value() {
        let text = format!("{CONTAINER}::2");
        assert_eq!(
            Err(IdError::Openehr {
                kind: "OBJECT_VERSION_ID",
                found: text.clone(),
                source: BaseIdError::PartCount {
                    expected: 3,
                    found: 2
                },
            }),
            version_from_etag(&text)
        );
    }

    #[test]
    fn a_weak_entity_tag_loses_its_indicator_and_quotes() {
        assert_eq!(
            "8849182c::system::1",
            entity_tag("W/\"8849182c::system::1\"")
        );
        assert_eq!("8849182c::system::1", entity_tag("\"8849182c::system::1\""));
        assert_eq!("8849182c::system::1", entity_tag("8849182c::system::1"));
    }

    #[test]
    fn a_version_id_parses_out_of_a_weak_entity_tag() -> Result<(), IdError> {
        let id = version_from_etag(&format!("W/\"{CONTAINER}::openEHRSys.example.com::1\""))?;
        assert_eq!("1", id.version_tree_id().value());
        Ok(())
    }

    #[test]
    fn a_request_id_refuses_a_newline() {
        assert!(RequestId::new("abc\ndef").is_err());
    }
}
