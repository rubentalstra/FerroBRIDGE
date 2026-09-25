// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Pointing the embedded DDL at a schema.
//!
//! OHDSI's rendered DDL writes every table as
//! `@cdmDatabaseSchema.<table>`, and the deployment decides the schema. The
//! substitution takes the schema name through [`SchemaName`], which admits
//! only an unquoted PostgreSQL identifier, so a name can never carry SQL of
//! its own into the statement.

use std::fmt;

/// The placeholder OHDSI's rendered DDL carries wherever the CDM schema is
/// named.
pub const SCHEMA_PLACEHOLDER: &str = "@cdmDatabaseSchema";

/// The longest identifier PostgreSQL keeps, in bytes.
///
/// PostgreSQL truncates an identifier at `NAMEDATALEN - 1`, 63 bytes in a
/// default build
/// (<https://www.postgresql.org/docs/16/sql-syntax-lexical.html#SQL-SYNTAX-IDENTIFIERS>),
/// so a longer name would silently become a different schema.
pub const MAX_IDENTIFIER_BYTES: usize = 63;

/// A schema name that is not an unquoted PostgreSQL identifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaNameError {
    /// The name is empty.
    #[error("the schema name is empty")]
    Empty,
    /// The name is longer than PostgreSQL keeps.
    #[error(
        "the schema name is {length} bytes, more than the {MAX_IDENTIFIER_BYTES} PostgreSQL keeps"
    )]
    TooLong {
        /// The length of the offending name, in bytes.
        length: usize,
    },
    /// The name has a byte an unquoted identifier cannot carry.
    #[error(
        "the schema name has `{byte}` at byte {index}, which an unquoted identifier cannot carry"
    )]
    Character {
        /// The offending byte, rendered as a character.
        byte: char,
        /// Where it sits in the name.
        index: usize,
    },
}

/// The name of the database schema the CDM tables live in.
///
/// The accepted form is the lower-case unquoted identifier OHDSI's DDL and the
/// CDM's own scripts use: a letter or an underscore, then letters, digits and
/// underscores, at most [`MAX_IDENTIFIER_BYTES`] of them. An upper-case name
/// is refused rather than folded, because PostgreSQL folds an unquoted
/// identifier to lower case and the caller would then be naming a schema it
/// did not write.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaName(String);

impl SchemaName {
    /// Creates a schema name, refusing anything that is not an unquoted
    /// PostgreSQL identifier.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaNameError`] when the name is empty, longer than
    /// [`MAX_IDENTIFIER_BYTES`] bytes, or carries anything outside
    /// `[a-z_][a-z0-9_]*`.
    pub fn new(name: impl Into<String>) -> Result<Self, SchemaNameError> {
        let name = name.into();
        if name.is_empty() {
            return Err(SchemaNameError::Empty);
        }
        if name.len() > MAX_IDENTIFIER_BYTES {
            return Err(SchemaNameError::TooLong { length: name.len() });
        }
        for (index, byte) in name.bytes().enumerate() {
            let allowed = match index {
                0 => byte.is_ascii_lowercase() || byte == b'_',
                _ => byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_',
            };
            if !allowed {
                return Err(SchemaNameError::Character {
                    byte: char::from(byte),
                    index,
                });
            }
        }
        Ok(Self(name))
    }

    /// Returns the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SchemaName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for SchemaName {
    type Err = SchemaNameError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::new(name)
    }
}

/// Returns `sql` with every [`SCHEMA_PLACEHOLDER`] replaced by `schema`.
///
/// # Examples
///
/// ```
/// use omop_cdm::{ddl, generated};
///
/// let statements = ddl::with_schema(generated::ddl::DDL, "cdm")?;
/// assert!(statements.contains("CREATE TABLE cdm.person"));
/// assert!(ddl::with_schema(generated::ddl::DDL, "public; drop").is_err());
/// # Ok::<(), omop_cdm::ddl::SchemaNameError>(())
/// ```
///
/// # Errors
///
/// Returns [`SchemaNameError`] when `schema` is not an unquoted PostgreSQL
/// identifier. Nothing is substituted in that case.
pub fn with_schema(sql: &str, schema: &str) -> Result<String, SchemaNameError> {
    Ok(in_schema(sql, &SchemaName::new(schema)?))
}

/// Returns `sql` with every [`SCHEMA_PLACEHOLDER`] replaced by a schema name
/// that is already checked.
#[must_use]
pub fn in_schema(sql: &str, schema: &SchemaName) -> String {
    sql.replace(SCHEMA_PLACEHOLDER, schema.as_str())
}
