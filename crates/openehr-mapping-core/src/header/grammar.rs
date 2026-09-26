// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `grammar` value: the mapping language and its version.

use core::fmt;
use core::str::FromStr;

/// One of the two mapping languages a header can declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MappingLanguage {
    /// FHIRconnect, the openEHR to HL7 FHIR mapping language.
    FhirConnect,
    /// OMOCL, the openEHR to OMOP Common Data Model mapping language.
    Omocl,
}

impl MappingLanguage {
    /// Returns the spelling the language's own schema or corpus writes.
    ///
    /// FHIRconnect's published schemas pin the spelling with the pattern
    /// `^FHIRConnect/v[0-9]+\.[0-9]+\.[0-9]+$`
    /// (`docs/specs/fhirconnect/build/site/FHIRconnect/v1.0.0/_attachments/model-mapping.schema.json`,
    /// `properties.grammar.pattern`); every OMOCL corpus file writes `OMOCL`.
    #[must_use]
    pub const fn canonical_spelling(self) -> &'static str {
        match self {
            Self::FhirConnect => "FHIRConnect",
            Self::Omocl => "OMOCL",
        }
    }
}

impl fmt::Display for MappingLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.canonical_spelling())
    }
}

/// The three-part version a `grammar` value carries after the `v`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrammarSemVer {
    major: u32,
    minor: u32,
    patch: u32,
}

impl GrammarSemVer {
    /// Creates a grammar version.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Returns the major version.
    #[must_use]
    pub const fn major(self) -> u32 {
        self.major
    }

    /// Returns the minor version.
    #[must_use]
    pub const fn minor(self) -> u32 {
        self.minor
    }

    /// Returns the patch version.
    #[must_use]
    pub const fn patch(self) -> u32 {
        self.patch
    }
}

impl fmt::Display for GrammarSemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Why a `grammar` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GrammarVersionError {
    /// The value carries no `/` separating the language from the version.
    #[error("`{value}` is not `<language>/v<major>.<minor>.<patch>`")]
    MissingSeparator {
        /// The refused value.
        value: String,
    },
    /// The language is neither FHIRconnect nor OMOCL.
    #[error("`{language}` is neither FHIRConnect nor OMOCL")]
    UnknownLanguage {
        /// The refused language spelling.
        language: String,
    },
    /// The version is outside the `vMAJOR.MINOR.PATCH` shape.
    #[error("`{version}` is not `v<major>.<minor>.<patch>`")]
    MalformedVersion {
        /// The refused version.
        version: String,
    },
}

/// The `grammar` value: a language and the version of its grammar.
///
/// The language spelling is compared case-insensitively and kept verbatim. The
/// specification's own text is case-inconsistent about its keyword values
/// (`docs/architecture.md` records the adjudication, and the FHIRconnect
/// schema pattern writes `FHIRConnect` where the specification title writes
/// FHIRconnect), so a case-exact comparison would refuse a file the
/// specification's own prose spells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarVersion {
    language: MappingLanguage,
    spelling: String,
    version: GrammarSemVer,
}

impl GrammarVersion {
    /// Returns the language.
    #[must_use]
    pub const fn language(&self) -> MappingLanguage {
        self.language
    }

    /// Returns the language spelling as the file writes it.
    #[must_use]
    pub fn spelling(&self) -> &str {
        &self.spelling
    }

    /// Returns the grammar version.
    #[must_use]
    pub const fn version(&self) -> GrammarSemVer {
        self.version
    }
}

impl fmt::Display for GrammarVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.spelling, self.version)
    }
}

impl FromStr for GrammarVersion {
    type Err = GrammarVersionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((spelling, version)) = value.split_once('/') else {
            return Err(GrammarVersionError::MissingSeparator {
                value: value.to_owned(),
            });
        };
        let language =
            if spelling.eq_ignore_ascii_case(MappingLanguage::FhirConnect.canonical_spelling()) {
                MappingLanguage::FhirConnect
            } else if spelling.eq_ignore_ascii_case(MappingLanguage::Omocl.canonical_spelling()) {
                MappingLanguage::Omocl
            } else {
                return Err(GrammarVersionError::UnknownLanguage {
                    language: spelling.to_owned(),
                });
            };
        let version =
            parse_grammar_semver(version).ok_or_else(|| GrammarVersionError::MalformedVersion {
                version: version.to_owned(),
            })?;
        Ok(Self {
            language,
            spelling: spelling.to_owned(),
            version,
        })
    }
}

/// Parses `v<major>.<minor>.<patch>`, the shape both grammar patterns write.
fn parse_grammar_semver(version: &str) -> Option<GrammarSemVer> {
    let digits = version.strip_prefix('v')?;
    let mut parts = digits.split('.');
    let major = parse_decimal(parts.next()?)?;
    let minor = parse_decimal(parts.next()?)?;
    let patch = parse_decimal(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(GrammarSemVer::new(major, minor, patch))
}

/// Parses one `[0-9]+` field of a grammar version.
fn parse_decimal(field: &str) -> Option<u32> {
    if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    field.parse().ok()
}
