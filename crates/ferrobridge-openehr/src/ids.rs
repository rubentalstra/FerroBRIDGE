// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The identifier newtypes the CDR wire carries.
//!
//! An openEHR identifier and a target-side identifier are different things, so
//! each one is its own type and a swapped argument is a compile error rather
//! than a mis-addressed request.

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
    /// The text is not `object_id::creating_system_id::version_tree_id`.
    #[error(
        "an OBJECT_VERSION_ID is object_id::creating_system_id::version_tree_id, {found:?} has {parts} parts"
    )]
    VersionIdShape {
        /// The text that was parsed.
        found: String,
        /// How many `::`-separated parts it carried.
        parts: usize,
    },
    /// The text is not an ADL 2 archetype HRID.
    #[error(
        "an ARCHETYPE_HRID is [namespace::]publisher-package-class.concept.vN[.M[.P]], {found:?} is not"
    )]
    HridShape {
        /// The text that was parsed.
        found: String,
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

/// A version container identifier, taken from `VERSIONED_OBJECT.uid.value`.
///
/// This is the `uid_based_id` in its `HIER_OBJECT_ID` form, which addresses
/// the latest version of a composition
/// (`ehr-codegen.openapi.yaml`, `components.parameters.uid_based_id`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionedObjectUid(String);

impl VersionedObjectUid {
    /// Returns the version container identifier `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is empty or carries a character that
    /// would change the request path.
    pub fn new(text: &str) -> Result<Self, IdError> {
        opaque("versioned_object_uid", text).map(Self)
    }

    /// Returns the identifier as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionedObjectUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A version identifier, taken from `VERSION.uid.value`.
///
/// RM Common §`OBJECT_VERSION_ID` gives the three-part form
/// `object_id::creating_system_id::version_tree_id`, of which only the leading
/// `object_id` (the [`VersionedObjectUid`]) is stable across updates.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[expect(
    clippy::struct_field_names,
    reason = "RM Common section OBJECT_VERSION_ID names all three parts with an id suffix"
)]
pub struct ObjectVersionId {
    /// The version container identifier.
    object_id: String,
    /// The identifier of the system that created this version.
    creating_system_id: String,
    /// The position of this version in its version tree.
    version_tree_id: String,
}

impl ObjectVersionId {
    /// Returns the version identifier `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is not three non-empty `::`-separated
    /// parts, or when a part carries a character that would change the request
    /// path.
    pub fn new(text: &str) -> Result<Self, IdError> {
        let parts: Vec<&str> = text.split("::").collect();
        let [object_id, creating_system_id, version_tree_id] = parts.as_slice() else {
            return Err(IdError::VersionIdShape {
                found: text.to_owned(),
                parts: parts.len(),
            });
        };
        Ok(Self {
            object_id: opaque("version_uid object_id", object_id)?,
            creating_system_id: opaque("version_uid creating_system_id", creating_system_id)?,
            version_tree_id: opaque("version_uid version_tree_id", version_tree_id)?,
        })
    }

    /// Returns the version identifier an `ETag` field value names.
    ///
    /// The `W/` weakness indicator and the double quotes ITS-REST 1.1.0
    /// §Requests and responses/HTTP headers/ETag and Last-Modified puts around
    /// the value are stripped before parsing.
    ///
    /// # Errors
    /// Returns [`IdError`] when what the entity tag carries is not an
    /// `OBJECT_VERSION_ID`.
    pub fn from_etag(value: &str) -> Result<Self, IdError> {
        Self::new(entity_tag(value))
    }

    /// Returns the version container this version belongs to.
    #[must_use]
    pub fn versioned_object_uid(&self) -> VersionedObjectUid {
        VersionedObjectUid(self.object_id.clone())
    }

    /// Returns the identifier of the system that created this version.
    #[must_use]
    pub fn creating_system_id(&self) -> &str {
        &self.creating_system_id
    }

    /// Returns the position of this version in its version tree.
    #[must_use]
    pub fn version_tree_id(&self) -> &str {
        &self.version_tree_id
    }
}

impl fmt::Display for ObjectVersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}::{}::{}",
            self.object_id, self.creating_system_id, self.version_tree_id
        )
    }
}

/// A CONTRIBUTION identifier, taken from `CONTRIBUTION.uid.value`.
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

/// An ADL 1.4 template identifier.
///
/// The ITS-REST 1.1.0 Definition API leaves the value opaque and its examples
/// range from `Vital Signs` to a full HRID
/// (`definition-codegen.openapi.yaml`, `components.parameters.template_id`),
/// so a space is legal and only path-structural characters are refused.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TemplateId(String);

impl TemplateId {
    /// Returns the template identifier `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is empty or carries a character that
    /// would change the request path.
    pub fn new(text: &str) -> Result<Self, IdError> {
        opaque("template_id", text).map(Self)
    }

    /// Returns the identifier as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TemplateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An ADL 2 archetype HRID, the identifier of an ADL 2 template.
///
/// The grammar is AM 2.4 §Identification: `namespaced_hrid = namespace '::'
/// local_hrid`, `local_hrid = hrid_root '.v' version_id`, `hrid_root =
/// rm_publisher '-' rm_closure '-' rm_class '.' concept_id`, with
/// `version_id = release_version [ '-' version_modifier '.' issue_number ]`.
/// A partial version resolves server-side to the latest matching major
/// version, so the identifier a fetch answers with is what a caller caches
/// under.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArchetypeHrid(String);

impl ArchetypeHrid {
    /// Returns the archetype HRID `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` does not match the AM 2.4
    /// §Identification HRID grammar.
    pub fn new(text: &str) -> Result<Self, IdError> {
        let shape = || IdError::HridShape {
            found: text.to_owned(),
        };
        let local = match text.split_once("::") {
            Some((namespace, local)) => {
                if namespace.is_empty() || local.is_empty() {
                    return Err(shape());
                }
                local
            }
            None => text,
        };
        let (root, rest) = local.split_once('.').ok_or_else(shape)?;
        let root_parts: Vec<&str> = root.split('-').collect();
        if root_parts.len() != 3 || root_parts.iter().any(|part| part.is_empty()) {
            return Err(shape());
        }
        let (concept, version) = rest.split_once('.').ok_or_else(shape)?;
        if concept.is_empty() {
            return Err(shape());
        }
        Self::check_version(version).ok_or_else(shape)?;
        opaque("archetype HRID", text).map(Self)
    }

    /// Returns the identifier of a decoded ADL 2 operational template.
    ///
    /// The physical form carries the complete version, which is what a partial
    /// request resolves to (AM 2.4 §Identification, `physical_id`).
    ///
    /// # Errors
    /// Returns [`IdError`] when the template's own identifier does not render
    /// as a well-formed HRID.
    pub fn from_aom2(
        hrid: &openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid,
    ) -> Result<Self, IdError> {
        Self::new(&hrid.physical_id())
    }

    /// Returns `Some(())` when `version` is `vN[.M[.P]]` with an optional
    /// `-modifier.issue` extension.
    fn check_version(version: &str) -> Option<()> {
        let (numbers, extension) = match version.split_once('-') {
            Some((numbers, extension)) => (numbers, Some(extension)),
            None => (version, None),
        };
        let numbers = numbers.strip_prefix('v')?;
        let mut parts = 0_usize;
        for part in numbers.split('.') {
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            parts = parts.saturating_add(1);
        }
        if !(1..=3).contains(&parts) {
            return None;
        }
        if extension.is_some_and(str::is_empty) {
            return None;
        }
        Some(())
    }

    /// Returns the identifier as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArchetypeHrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The subject identifier an EHR lookup matches against
/// `EHR_STATUS.subject.external_ref.id.value`.
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
        ArchetypeHrid, EhrId, IdError, ObjectVersionId, RequestId, TemplateId, entity_tag,
    };

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
        assert_eq!("Vital Signs", TemplateId::new("Vital Signs")?.as_str());
        Ok(())
    }

    #[test]
    fn a_version_id_splits_into_its_three_parts() -> Result<(), IdError> {
        let id = ObjectVersionId::new("8849182c::openEHRSys.example.com::2")?;
        assert_eq!("8849182c", id.versioned_object_uid().as_str());
        assert_eq!("openEHRSys.example.com", id.creating_system_id());
        assert_eq!("2", id.version_tree_id());
        assert_eq!("8849182c::openEHRSys.example.com::2", id.to_string());
        Ok(())
    }

    #[test]
    fn a_version_id_refuses_a_two_part_value() {
        assert_eq!(
            Err(IdError::VersionIdShape {
                found: "8849182c::2".to_owned(),
                parts: 2
            }),
            ObjectVersionId::new("8849182c::2")
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
        let id = ObjectVersionId::from_etag("W/\"8849182c::openEHRSys.example.com::1\"")?;
        assert_eq!("1", id.version_tree_id());
        Ok(())
    }

    #[test]
    fn an_hrid_accepts_the_namespaced_three_part_version() -> Result<(), IdError> {
        let hrid = "org.highmed::openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0";
        assert_eq!(hrid, ArchetypeHrid::new(hrid)?.as_str());
        Ok(())
    }

    #[test]
    fn an_hrid_accepts_a_partial_version_and_a_release_candidate() -> Result<(), IdError> {
        ArchetypeHrid::new("openEHR-EHR-COMPOSITION.t_vital_signs.v1")?;
        ArchetypeHrid::new("openEHR-EHR-COMPOSITION.t_vital_signs.v1.8.2-rc.4")?;
        Ok(())
    }

    #[test]
    fn an_hrid_refuses_a_missing_version_marker() {
        assert!(ArchetypeHrid::new("openEHR-EHR-COMPOSITION.t_vital_signs.1.0.0").is_err());
    }

    #[test]
    fn an_hrid_refuses_a_two_part_root() {
        assert!(ArchetypeHrid::new("openEHR-COMPOSITION.t_vital_signs.v1.0.0").is_err());
    }

    #[test]
    fn a_request_id_refuses_a_newline() {
        assert!(RequestId::new("abc\ndef").is_err());
    }
}
