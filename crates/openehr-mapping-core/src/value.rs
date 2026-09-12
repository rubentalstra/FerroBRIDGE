// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The positioned YAML value tree a mapping file loads into.
//!
//! Every node carries the line and column of the YAML it came from, so a
//! refusal anywhere downstream can name the place in the file that caused it.
//! Anchors, aliases and merge keys are already resolved when a tree reaches a
//! reader: an aliased node carries the position of the alias, which is where a
//! reader of the file looks first.

use core::fmt;
use std::collections::BTreeMap;

use serde::de::Deserialize;
use serde::de::Deserializer;
use serde::de::Error;
use serde::de::MapAccess;
use serde::de::SeqAccess;
use serde::de::Visitor;

use crate::position::Position;

/// A YAML float, compared by its bit pattern so the value tree is `Eq`.
///
/// Bit equality is a total equivalence over `f64`, which the usual numeric
/// comparison is not: `NaN != NaN` would break reflexivity. The loader rejects
/// a non-finite float before one reaches this type, so the only visible
/// difference from numeric comparison is that `0.0` and `-0.0` are distinct.
#[derive(Debug, Clone, Copy)]
pub struct Float(f64);

impl Float {
    /// Wraps a float read from a YAML scalar.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Returns the float.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl PartialEq for Float {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Float {}

impl fmt::Display for Float {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The kind of a YAML node, for a diagnostic that has to say what it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueKind {
    /// The YAML null.
    Null,
    /// A boolean.
    Bool,
    /// A number.
    Number,
    /// A string.
    Text,
    /// A sequence.
    Sequence,
    /// A mapping.
    Mapping,
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match *self {
            Self::Null => "null",
            Self::Bool => "a boolean",
            Self::Number => "a number",
            Self::Text => "a string",
            Self::Sequence => "a sequence",
            Self::Mapping => "a mapping",
        };
        f.write_str(name)
    }
}

/// One resolved YAML node.
///
/// Numbers keep the widest form the parser resolved them into rather than one
/// common numeric type, so a 64-bit OMOP `concept_id` survives the load
/// unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingValue {
    /// The YAML null, and the value of a key written with no value at all.
    Null,
    /// A boolean.
    Bool(bool),
    /// A negative integer.
    Signed(i64),
    /// A non-negative integer.
    Unsigned(u64),
    /// A finite float.
    Float(Float),
    /// A string.
    Text(String),
    /// A sequence, in document order.
    Sequence(Vec<PositionedValue>),
    /// A mapping, keyed by the string key, in key order.
    Mapping(BTreeMap<String, MappingEntry>),
}

impl MappingValue {
    /// Returns the kind of this node.
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match *self {
            Self::Null => ValueKind::Null,
            Self::Bool(_) => ValueKind::Bool,
            Self::Signed(_) | Self::Unsigned(_) | Self::Float(_) => ValueKind::Number,
            Self::Text(_) => ValueKind::Text,
            Self::Sequence(_) => ValueKind::Sequence,
            Self::Mapping(_) => ValueKind::Mapping,
        }
    }
}

/// A YAML node paired with the position it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionedValue {
    position: Position,
    value: MappingValue,
}

impl PositionedValue {
    /// Pairs a node with the position it was read from.
    #[must_use]
    pub const fn new(position: Position, value: MappingValue) -> Self {
        Self { position, value }
    }

    /// Returns the position the node was read from.
    #[must_use]
    pub const fn position(&self) -> Position {
        self.position
    }

    /// Returns the node.
    #[must_use]
    pub const fn value(&self) -> &MappingValue {
        &self.value
    }

    /// Returns the kind of the node.
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        self.value.kind()
    }

    /// Returns the string this node holds, or `None` when it holds anything
    /// else.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self.value {
            MappingValue::Text(ref text) => Some(text),
            _ => None,
        }
    }

    /// Returns the sequence this node holds, or `None` when it holds anything
    /// else.
    #[must_use]
    pub fn as_sequence(&self) -> Option<&[Self]> {
        match self.value {
            MappingValue::Sequence(ref items) => Some(items),
            _ => None,
        }
    }

    /// Returns the mapping this node holds, or `None` when it holds anything
    /// else.
    #[must_use]
    pub fn as_mapping(&self) -> Option<&BTreeMap<String, MappingEntry>> {
        match self.value {
            MappingValue::Mapping(ref entries) => Some(entries),
            _ => None,
        }
    }

    /// Returns the entry this node holds under `key`, or `None` when the node
    /// is not a mapping or has no such key.
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&MappingEntry> {
        self.as_mapping().and_then(|entries| entries.get(key))
    }

    /// Returns the value this node holds under `key`, or `None` when the node
    /// is not a mapping or has no such key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        self.entry(key).map(MappingEntry::value)
    }
}

/// One key and value of a YAML mapping.
///
/// The key carries its own position, because a diagnostic about a missing or
/// misspelled key points at the key rather than at the value beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingEntry {
    key_position: Position,
    value: PositionedValue,
}

impl MappingEntry {
    /// Pairs a key position with the value written under that key.
    #[must_use]
    pub const fn new(key_position: Position, value: PositionedValue) -> Self {
        Self {
            key_position,
            value,
        }
    }

    /// Returns the position of the key.
    #[must_use]
    pub const fn key_position(&self) -> Position {
        self.key_position
    }

    /// Returns the value written under the key.
    #[must_use]
    pub const fn value(&self) -> &PositionedValue {
        &self.value
    }
}

impl<'de> Deserialize<'de> for PositionedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let spanned = serde_saphyr::Spanned::<MappingValue>::deserialize(deserializer)?;
        Ok(Self::new(Position::from(spanned.referenced), spanned.value))
    }
}

impl<'de> Deserialize<'de> for MappingValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(MappingValueVisitor)
    }
}

struct MappingValueVisitor;

impl<'de> Visitor<'de> for MappingValueVisitor {
    type Value = MappingValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any YAML node")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(MappingValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(MappingValue::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(MappingValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(MappingValue::Signed(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(MappingValue::Unsigned(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(MappingValue::Float(Float::new(value)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(MappingValue::Text(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: Error,
    {
        Ok(MappingValue::Text(value))
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::with_capacity(access.size_hint().unwrap_or_default());
        while let Some(item) = access.next_element::<PositionedValue>()? {
            items.push(item);
        }
        Ok(MappingValue::Sequence(items))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = BTreeMap::new();
        while let Some(key) = access.next_key::<serde_saphyr::Spanned<String>>()? {
            let value = access.next_value::<PositionedValue>()?;
            let entry = MappingEntry::new(Position::from(key.referenced), value);
            if entries.insert(key.value.clone(), entry).is_some() {
                return Err(A::Error::custom(format!(
                    "duplicate mapping key `{}`",
                    key.value
                )));
            }
        }
        Ok(MappingValue::Mapping(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::Float;
    use super::MappingValue;
    use super::ValueKind;

    #[test]
    fn float_equality_is_by_bit_pattern() {
        assert_eq!(Float::new(5.4), Float::new(5.4));
        assert_ne!(Float::new(0.0), Float::new(-0.0));
    }

    #[test]
    fn a_node_reports_its_kind() {
        assert_eq!(MappingValue::Null.kind(), ValueKind::Null);
        assert_eq!(MappingValue::Unsigned(4).kind(), ValueKind::Number);
        assert_eq!(
            MappingValue::Float(Float::new(5.4)).kind(),
            ValueKind::Number
        );
        assert_eq!(MappingValue::Text("R4".to_owned()).kind(), ValueKind::Text);
    }

    #[test]
    fn a_kind_renders_for_a_diagnostic() {
        assert_eq!(ValueKind::Mapping.to_string(), "a mapping");
        assert_eq!(ValueKind::Null.to_string(), "null");
    }
}
