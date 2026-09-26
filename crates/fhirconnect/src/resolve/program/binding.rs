// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a program was compiled against: the profile, the template and the
//! version of every model mapping.

use core::fmt;

use openehr_mapping_core::header::archetype::ArchetypeId;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::header::metadata::MappingVersion;
use openehr_mapping_core::template::Generation;

/// The canonical URL of a FHIR profile, as a context mapping writes it.
///
/// A canonical URL may carry a version after a `|`
/// (<https://hl7.org/fhir/R4/references.html#canonical>); this type holds the
/// URL alone, and the version travels beside it in [`ProfileBinding`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileUrl(String);

impl ProfileUrl {
    /// Wraps a profile canonical URL.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// Returns the URL.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The identifier of an operational template, as a context mapping writes it.
///
/// This is the openEHR side of the selection and never carries a FHIR
/// identifier, so a swapped argument is a compile error rather than a wrong
/// program.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TemplateId(String);

impl TemplateId {
    /// Wraps a template identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the identifier.
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

/// The FHIR resource type a program reads and writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceType(String);

impl ResourceType {
    /// Wraps a resource type name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the resource type name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A version selector, which a mapping file may leave out.
///
/// The context schema makes `profile.version` and `template.sem_ver` optional,
/// so a program records what the files pinned and says so where they pinned
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pin {
    /// The file names a version, and the compiler checked what it could.
    Pinned(String),
    /// The file names no version.
    Unpinned,
}

impl Pin {
    /// Wraps an optional version.
    #[must_use]
    pub fn new(version: Option<&str>) -> Self {
        version.map_or(Self::Unpinned, |value| Self::Pinned(value.to_owned()))
    }

    /// Returns the pinned version, `None` when the file pinned none.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        match *self {
            Self::Pinned(ref version) => Some(version),
            Self::Unpinned => None,
        }
    }

    /// Whether the file pinned no version.
    #[must_use]
    pub const fn is_unpinned(&self) -> bool {
        matches!(*self, Self::Unpinned)
    }
}

impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Pinned(ref version) => f.write_str(version),
            Self::Unpinned => f.write_str("unpinned"),
        }
    }
}

/// The profile a program maps, as the context mapping pins it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileBinding {
    url: ProfileUrl,
    version: Pin,
}

impl ProfileBinding {
    /// Binds a profile URL to the version the context pins.
    #[must_use]
    pub const fn new(url: ProfileUrl, version: Pin) -> Self {
        Self { url, version }
    }

    /// Returns the profile URL.
    #[must_use]
    pub const fn url(&self) -> &ProfileUrl {
        &self.url
    }

    /// Returns the pinned profile version.
    #[must_use]
    pub const fn version(&self) -> &Pin {
        &self.version
    }
}

impl fmt::Display for ProfileBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} version {}", self.url, self.version)
    }
}

/// The operational template a program maps, as the context mapping pins it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateBinding {
    id: TemplateId,
    sem_ver: Pin,
    generation: Generation,
}

impl TemplateBinding {
    /// Binds a template identifier to the version the context pins.
    #[must_use]
    pub const fn new(id: TemplateId, sem_ver: Pin, generation: Generation) -> Self {
        Self {
            id,
            sem_ver,
            generation,
        }
    }

    /// Returns the template identifier.
    #[must_use]
    pub const fn id(&self) -> &TemplateId {
        &self.id
    }

    /// Returns the pinned template version.
    #[must_use]
    pub const fn sem_ver(&self) -> &Pin {
        &self.sem_ver
    }

    /// Returns the generation the template was served in.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }
}

impl fmt::Display for TemplateBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} sem_ver {} generation {}",
            self.id, self.sem_ver, self.generation
        )
    }
}

/// One model mapping a program compiled, with the versions it was compiled
/// against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelBinding {
    name: MappingName,
    version: MappingVersion,
    archetype: Option<ArchetypeId>,
    revision: Pin,
    extensions: Vec<MappingName>,
}

impl ModelBinding {
    /// Records one compiled model mapping.
    #[must_use]
    pub const fn new(
        name: MappingName,
        version: MappingVersion,
        archetype: Option<ArchetypeId>,
        revision: Pin,
        extensions: Vec<MappingName>,
    ) -> Self {
        Self {
            name,
            version,
            archetype,
            revision,
            extensions,
        }
    }

    /// Returns the `metadata.name` of the model mapping.
    #[must_use]
    pub const fn name(&self) -> &MappingName {
        &self.name
    }

    /// Returns the `metadata.version` the compiled file carried.
    #[must_use]
    pub const fn version(&self) -> &MappingVersion {
        &self.version
    }

    /// Returns the archetype the model mapping declares, which an extension
    /// file declares none of.
    #[must_use]
    pub const fn archetype(&self) -> Option<&ArchetypeId> {
        self.archetype.as_ref()
    }

    /// Returns the archetype revision the model mapping pins.
    #[must_use]
    pub const fn revision(&self) -> &Pin {
        &self.revision
    }

    /// Returns the extensions applied to it, in the order they applied.
    #[must_use]
    pub fn extensions(&self) -> &[MappingName] {
        &self.extensions
    }
}
