// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The keyword values of the grammar, each parsed against its documented set.

use core::fmt;
use core::str::FromStr;

/// Why a keyword value was refused.
///
/// The message lists every value the keyword admits, so a mapping author sees
/// the whole set rather than only that the value was wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not one of {}", render_admitted(admitted))]
pub struct KeywordError {
    /// The refused value, as the file writes it.
    pub value: String,
    /// Every value the keyword admits.
    pub admitted: &'static [&'static str],
}

/// Renders an admitted set as a comma-separated backticked list.
fn render_admitted(admitted: &[&str]) -> String {
    admitted
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// The direction a `unidirectional` key pins a file or a mapping to.
///
/// The two spellings are fixed by the header page ("`unidirectional` can be
/// set in the `spec` with `\"openehr->fhir\"` or `\"fhir->openehr\"`",
/// `docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Direction)
/// and repeated for a mapping method in
/// `docs/specs/fhirconnect/modules/ROOT/pages/basics/body.adoc`, §Undirectional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// The file or mapping runs openEHR to FHIR only.
    OpenehrToFhir,
    /// The file or mapping runs FHIR to openEHR only.
    FhirToOpenehr,
}

impl Direction {
    /// Every spelling the specification writes.
    pub const ADMITTED: &'static [&'static str] = &["openehr->fhir", "fhir->openehr"];

    /// Returns the spelling the specification writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenehrToFhir => "openehr->fhir",
            Self::FhirToOpenehr => "fhir->openehr",
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Direction {
    type Err = KeywordError;

    /// Parses the value case-insensitively inside the documented set.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a value outside the two spellings.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case(Self::OpenehrToFhir.as_str()) {
            Ok(Self::OpenehrToFhir)
        } else if value.eq_ignore_ascii_case(Self::FhirToOpenehr.as_str()) {
            Ok(Self::FhirToOpenehr)
        } else {
            Err(KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            })
        }
    }
}

/// How an extension mapping interacts with the model mapping it extends.
///
/// The three methods are fixed by
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`
/// and by the `properties.extension.enum` of the published model schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionMethod {
    /// Adds a mapping at the bottom of the model mapping.
    Add,
    /// Appends a `followedBy` to the mapping method `appendTo` names.
    Append,
    /// Replaces the mapping method of the same name.
    Overwrite,
}

impl ExtensionMethod {
    /// Every spelling the schema enum admits.
    pub const ADMITTED: &'static [&'static str] = &["add", "append", "overwrite"];

    /// Returns the spelling the schema enum fixes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Append => "append",
            Self::Overwrite => "overwrite",
        }
    }
}

impl fmt::Display for ExtensionMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExtensionMethod {
    type Err = KeywordError;

    /// Parses the value case-exactly.
    ///
    /// The published model schema fixes the value with a JSON Schema `enum`
    /// (`properties.extension.enum`), and a JSON Schema enum matches
    /// case-sensitively, so `Add` is not this value.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a value outside the three methods.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "add" => Ok(Self::Add),
            "append" => Ok(Self::Append),
            "overwrite" => Ok(Self::Overwrite),
            _ => Err(KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            }),
        }
    }
}

/// The data type a `with` block pins the pair of paths to.
///
/// The set is the `enum` of the published model schema
/// (`#/$defs/mapping/properties/type/enum`); the deprecated static-typing
/// table in
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/data-mappings.adoc`
/// names the same identifiers, and `NONE` is the iteration marker described in
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataType {
    /// No transformation; the mapping iterates so its children can map.
    None,
    /// A FHIR Quantity.
    Quantity,
    /// A FHIR dateTime.
    DateTime,
    /// A FHIR `CodeableConcept`.
    CodeableConcept,
    /// A FHIR Coding.
    Coding,
    /// A FHIR string.
    String,
    /// A FHIR Dosage.
    Dosage,
    /// A FHIR id.
    Id,
    /// A FHIR Identifier.
    Identifier,
    /// A FHIR Ratio, against an openEHR `DV_PROPORTION`.
    Proportion,
}

impl DataType {
    /// Every spelling the schema enum admits.
    pub const ADMITTED: &'static [&'static str] = &[
        "NONE",
        "QUANTITY",
        "DATETIME",
        "CODEABLECONCEPT",
        "CODING",
        "STRING",
        "DOSAGE",
        "ID",
        "IDENTIFIER",
        "PROPORTION",
    ];

    /// Returns the spelling the schema enum fixes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Quantity => "QUANTITY",
            Self::DateTime => "DATETIME",
            Self::CodeableConcept => "CODEABLECONCEPT",
            Self::Coding => "CODING",
            Self::String => "STRING",
            Self::Dosage => "DOSAGE",
            Self::Id => "ID",
            Self::Identifier => "IDENTIFIER",
            Self::Proportion => "PROPORTION",
        }
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DataType {
    type Err = KeywordError;

    /// Parses the value case-exactly, as the schema `enum` fixes it.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a value outside the ten data types.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "NONE" => Ok(Self::None),
            "QUANTITY" => Ok(Self::Quantity),
            "DATETIME" => Ok(Self::DateTime),
            "CODEABLECONCEPT" => Ok(Self::CodeableConcept),
            "CODING" => Ok(Self::Coding),
            "STRING" => Ok(Self::String),
            "DOSAGE" => Ok(Self::Dosage),
            "ID" => Ok(Self::Id),
            "IDENTIFIER" => Ok(Self::Identifier),
            "PROPORTION" => Ok(Self::Proportion),
            _ => Err(KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            }),
        }
    }
}

/// The test a condition applies to the values under its `targetAttributes`.
///
/// The five operators and their combining logic come from the operator table
/// in `docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConditionOperator {
    /// The path holds one of the criteria; the criteria combine with OR.
    OneOf,
    /// The path holds none of the criteria; the criteria combine with AND.
    NotOf,
    /// The path is empty; it takes no criteria.
    Empty,
    /// The path is not empty; it takes no criteria.
    NotEmpty,
    /// The element at the path is of the type named in the criteria.
    Type,
}

impl ConditionOperator {
    /// Every spelling the operator table writes.
    pub const ADMITTED: &'static [&'static str] =
        &["one of", "not of", "empty", "not empty", "type"];

    /// Returns the spelling the operator table writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneOf => "one of",
            Self::NotOf => "not of",
            Self::Empty => "empty",
            Self::NotEmpty => "not empty",
            Self::Type => "type",
        }
    }

    /// Whether the operator takes a `criteria`.
    ///
    /// "`criteria` is not required for `empty` or `not empty`"
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
    /// §criteria), and both operator rows say the operator "does not include a
    /// `criteria`".
    #[must_use]
    pub const fn takes_criteria(self) -> bool {
        match self {
            Self::OneOf | Self::NotOf | Self::Type => true,
            Self::Empty | Self::NotEmpty => false,
        }
    }
}

impl fmt::Display for ConditionOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ConditionOperator {
    type Err = KeywordError;

    /// Parses the value case-exactly, as the operator table spells it.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a value outside the five operators.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "one of" => Ok(Self::OneOf),
            "not of" => Ok(Self::NotOf),
            "empty" => Ok(Self::Empty),
            "not empty" => Ok(Self::NotEmpty),
            "type" => Ok(Self::Type),
            _ => Err(KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            }),
        }
    }
}

/// A `$name` variable at the head of a FHIRconnect path.
///
/// The seven variables are the rows of the variable table in
/// `docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Variable {
    /// The root of the FHIR resource.
    Resource,
    /// The parent FHIR path of the enclosing mapping step.
    FhirRoot,
    /// The root of the openEHR archetype.
    Archetype,
    /// The parent openEHR path of the enclosing mapping step.
    OpenehrRoot,
    /// The root of the enclosing openEHR composition.
    Composition,
    /// The marker for a path skipped because a reference resolves it.
    Reference,
    /// The values passed in on the REST call, in operational mappings.
    Context,
}

impl Variable {
    /// Every variable name the table lists, without the leading `$`.
    pub const ADMITTED: &'static [&'static str] = &[
        "resource",
        "fhirRoot",
        "archetype",
        "openehrRoot",
        "composition",
        "reference",
        "context",
    ];

    /// Returns the name the table writes, without the leading `$`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::FhirRoot => "fhirRoot",
            Self::Archetype => "archetype",
            Self::OpenehrRoot => "openehrRoot",
            Self::Composition => "composition",
            Self::Reference => "reference",
            Self::Context => "context",
        }
    }
}

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}", self.as_str())
    }
}

impl FromStr for Variable {
    type Err = KeywordError;

    /// Parses a variable name, with or without the leading `$`,
    /// case-insensitively inside the documented set.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a name outside the seven variables.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let name = value.strip_prefix('$').unwrap_or(value);
        let admitted = [
            Self::Resource,
            Self::FhirRoot,
            Self::Archetype,
            Self::OpenehrRoot,
            Self::Composition,
            Self::Reference,
            Self::Context,
        ];
        admitted
            .into_iter()
            .find(|variable| name.eq_ignore_ascii_case(variable.as_str()))
            .ok_or_else(|| KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            })
    }
}
