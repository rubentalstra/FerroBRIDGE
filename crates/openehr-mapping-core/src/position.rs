// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Source positions, and values paired with the position they were read from.

use core::fmt;

/// A line and column in a YAML source document, both 1-based.
///
/// The YAML parser reports character-based, 1-indexed coordinates
/// (<https://docs.rs/serde-saphyr/1.2.0/serde_saphyr/struct.Location.html>),
/// and this type carries them unchanged so a diagnostic points at the same
/// place an editor does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    line: u64,
    column: u64,
}

impl Position {
    /// Creates a position from a 1-based line and column.
    #[must_use]
    pub const fn new(line: u64, column: u64) -> Self {
        Self { line, column }
    }

    /// Returns the 1-based line.
    #[must_use]
    pub const fn line(self) -> u64 {
        self.line
    }

    /// Returns the 1-based column.
    #[must_use]
    pub const fn column(self) -> u64 {
        self.column
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

impl From<serde_saphyr::Location> for Position {
    fn from(location: serde_saphyr::Location) -> Self {
        Self::new(location.line(), location.column())
    }
}

/// A value paired with the position of the YAML node it was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Located<T> {
    position: Position,
    value: T,
}

impl<T> Located<T> {
    /// Pairs a value with the position it was read from.
    #[must_use]
    pub const fn new(position: Position, value: T) -> Self {
        Self { position, value }
    }

    /// Returns the position the value was read from.
    #[must_use]
    pub const fn position(&self) -> Position {
        self.position
    }

    /// Returns the value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Consumes the pairing and returns the value.
    #[must_use]
    pub fn into_value(self) -> T {
        self.value
    }
}

impl<T> AsRef<T> for Located<T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

#[cfg(test)]
mod tests {
    use super::Located;
    use super::Position;

    #[test]
    fn a_position_renders_as_line_and_column() {
        assert_eq!(Position::new(3, 17).to_string(), "3:17");
    }

    #[test]
    fn a_located_value_keeps_both_halves() {
        let located = Located::new(Position::new(2, 5), "model");
        assert_eq!(located.position(), Position::new(2, 5));
        assert_eq!(*located.value(), "model");
        assert_eq!(located.into_value(), "model");
    }
}
