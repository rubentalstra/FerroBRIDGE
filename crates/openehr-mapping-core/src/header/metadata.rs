// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `metadata.name`, `metadata.version` and `type` values.

use core::fmt;
use core::str::FromStr;

/// Why a `metadata.name` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MappingNameError {
    /// The name was empty.
    #[error("a mapping name is not empty")]
    Empty,
    /// The name has leading or trailing whitespace.
    #[error("the mapping name `{name}` has leading or trailing whitespace")]
    Untrimmed {
        /// The refused name.
        name: String,
    },
}

/// The `metadata.name` value: the id a mapping is referenced by.
///
/// The FHIRconnect header page calls the name "a unique id used to identify
/// the mapping and reference it"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Metadata),
/// and every cross-file reference resolves against it by exact string. The
/// published schemas constrain the value only to a string, so refusing an
/// empty or whitespace-padded name is FerroBRIDGE's own rule: neither can be
/// referenced unambiguously.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MappingName(String);

impl MappingName {
    /// Creates a mapping name.
    ///
    /// # Errors
    ///
    /// Returns [`MappingNameError`] when the name is empty or carries leading
    /// or trailing whitespace.
    pub fn new(name: impl Into<String>) -> Result<Self, MappingNameError> {
        let name = name.into();
        if name.is_empty() {
            return Err(MappingNameError::Empty);
        }
        if name.trim() != name {
            return Err(MappingNameError::Untrimmed { name });
        }
        Ok(Self(name))
    }

    /// Returns the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MappingName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The `metadata.version` value, kept exactly as the file writes it.
///
/// Both published schemas type the field as a plain string with no pattern
/// (`model-mapping.schema.json`, `properties.metadata.properties.version`), so
/// no version grammar is imposed on it here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MappingVersion(String);

impl MappingVersion {
    /// Creates a mapping version from its lexical form.
    #[must_use]
    pub fn new(version: impl Into<String>) -> Self {
        Self(version.into())
    }

    /// Returns the lexical form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MappingVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a `type` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not `model`, `extension` or `context`")]
pub struct MappingTypeError {
    /// The refused value.
    pub value: String,
}

/// The `type` value: which of the three mapping-file kinds this file is.
///
/// FHIRconnect defines exactly three
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/main.adoc`):
/// the model mapping between an archetype and an unprofiled resource, the
/// extension mapping that adds to a model mapping for a profile and template,
/// and the context mapping that imports the other two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MappingType {
    /// An archetype to unprofiled-resource mapping.
    Model,
    /// An addition to a model mapping, for one profile and template.
    Extension,
    /// The import file that names a profile, a template and a starting
    /// mapping.
    Context,
}

impl MappingType {
    /// Returns the spelling the schema enum fixes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Extension => "extension",
            Self::Context => "context",
        }
    }
}

impl fmt::Display for MappingType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MappingType {
    type Err = MappingTypeError;

    /// Parses the value case-exactly.
    ///
    /// Both published schemas fix the value with a JSON Schema `enum`
    /// (`model-mapping.schema.json`, `properties.type.enum`), and a JSON
    /// Schema enum matches case-sensitively, so `Model` is not this value.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "model" => Ok(Self::Model),
            "extension" => Ok(Self::Extension),
            "context" => Ok(Self::Context),
            _ => Err(MappingTypeError {
                value: value.to_owned(),
            }),
        }
    }
}
