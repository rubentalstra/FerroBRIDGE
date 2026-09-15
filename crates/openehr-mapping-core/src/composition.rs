// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Composition build and read, behind one path and value seam.
//!
//! Both mapping engines write and read openEHR content as a set of template
//! nodes with values, so both reach a composition through this one seam: a
//! list of [`NodeValue`] builds a canonical composition, and a resolved node
//! plus its occurrences reads one value back out of it. Canonical JSON is the
//! mandatory composition representation of ITS-REST 1.1.0, so it is what
//! crosses the CDR wire in both directions.
//!
//! Two index bases meet here and are separate types because confusing them
//! silently mis-keys a value: an RM positional predicate is 1-based (openEHR
//! BASE Release 1.2.0 §Paths and Locators) and a FLAT instance index is
//! 0-based (Simplified Formats, §Instance Indexing).

use openehr_rm::v1_2::paths::RmPath;
use serde_json::Value;

use crate::index::FlatId;
use crate::index::ResolvedNode;
use crate::index::WebTemplateIndex;
use crate::template::Generation;
use crate::template::PathError;

/// The 1-based position of one instance, as an openEHR path writes it.
///
/// openEHR BASE Release 1.2.0 §Paths and Locators gives the positional
/// predicate as the `XPath` one, whose first item is `[1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RmPosition(u32);

impl RmPosition {
    /// Creates a position.
    ///
    /// # Errors
    ///
    /// Returns [`PositionError::Zero`] when `position` is 0, which no openEHR
    /// path may write.
    pub const fn new(position: u32) -> Result<Self, PositionError> {
        if position == 0 {
            return Err(PositionError::Zero);
        }
        Ok(Self(position))
    }

    /// Returns the position of the first instance.
    #[must_use]
    pub const fn first() -> Self {
        Self(1)
    }

    /// Returns the position.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl core::fmt::Display for RmPosition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The 0-based index of one instance, as a FLAT key writes it.
///
/// The Simplified Formats specification gives the instance index of a FLAT key
/// as zero-based (§Instance Indexing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlatIndex(u32);

impl FlatIndex {
    /// Creates an index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl core::fmt::Display for FlatIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<RmPosition> for FlatIndex {
    /// Converts a 1-based RM position into the 0-based FLAT index of the same
    /// instance.
    fn from(position: RmPosition) -> Self {
        Self(position.0.saturating_sub(1))
    }
}

impl TryFrom<FlatIndex> for RmPosition {
    type Error = PositionError;

    /// Converts a 0-based FLAT index into the 1-based RM position of the same
    /// instance.
    ///
    /// # Errors
    ///
    /// Returns [`PositionError::Overflow`] for the last representable index,
    /// which has no position.
    fn try_from(index: FlatIndex) -> Result<Self, Self::Error> {
        index
            .0
            .checked_add(1)
            .map(Self)
            .ok_or(PositionError::Overflow)
    }
}

/// Why an instance index could not be read as a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PositionError {
    /// The position was 0, and openEHR positions start at 1.
    #[error("an openEHR positional predicate starts at 1")]
    Zero,
    /// The index is the last representable one, so it has no position.
    #[error("the instance index has no 1-based position")]
    Overflow,
}

/// One value to write, at one node of the template.
///
/// The occurrences name the instance of each repeating node on the way to the
/// target, outermost first, and the datum part names which part of a data
/// value the value is (Simplified Formats, §Attribute Suffixes). A node with
/// one unsuffixed value carries no datum part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeValue {
    /// The node the value belongs to.
    flat_id: FlatId,
    /// The instance of each repeating node on the way to the target.
    occurrences: Vec<RmPosition>,
    /// The value-internal family below the node, outermost first.
    sub_path: Vec<String>,
    /// The datum part of the value, when it is one part of a data value.
    datum: Option<String>,
    /// The value itself.
    value: Value,
}

impl NodeValue {
    /// Creates a value at `node`, with no occurrences and no datum part.
    #[must_use]
    pub fn new(node: &ResolvedNode, value: Value) -> Self {
        Self {
            flat_id: node.flat_id().clone(),
            occurrences: Vec::new(),
            sub_path: Vec::new(),
            datum: None,
            value,
        }
    }

    /// Returns this value under a value-internal family of its node.
    ///
    /// Not every part of a data value is a datum suffix: an interval writes
    /// `lower` and `upper`, and a term mapping writes `_mapping:0/target`,
    /// which Simplified Formats models as paths below the node (§§
    /// `DV_INTERVAL`, `TERM_MAPPING`). Each segment is written as it stands,
    /// so an indexed family carries its own `:i`.
    #[must_use]
    pub fn under(mut self, sub_path: Vec<String>) -> Self {
        self.sub_path = sub_path;
        self
    }

    /// Returns this value with the instance of each repeating node on the way
    /// to its own, outermost first.
    #[must_use]
    pub fn with_occurrences(mut self, occurrences: Vec<RmPosition>) -> Self {
        self.occurrences = occurrences;
        self
    }

    /// Returns this value as the named part of a data value.
    #[must_use]
    pub fn with_datum(mut self, datum: impl Into<String>) -> Self {
        self.datum = Some(datum.into());
        self
    }

    /// Returns the node the value belongs to.
    #[must_use]
    pub const fn flat_id(&self) -> &FlatId {
        &self.flat_id
    }

    /// Returns the value itself.
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }
}

/// A composition in the canonical JSON of ITS-REST 1.1.0.
///
/// The template identifier and the generation travel with the document,
/// because reading a value back out of it needs the index of the same
/// template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalComposition {
    /// The canonical JSON of the composition.
    value: Value,
    /// The template the composition was built against.
    template_id: String,
    /// The generation of that template.
    generation: Generation,
}

impl CanonicalComposition {
    /// Wraps canonical JSON a CDR served as a composition of `template_id`.
    #[must_use]
    pub fn new(value: Value, template_id: impl Into<String>, generation: Generation) -> Self {
        Self {
            value,
            template_id: template_id.into(),
            generation,
        }
    }

    /// Returns the canonical JSON of the composition.
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }

    /// Returns the canonical JSON of the composition, consuming it.
    #[must_use]
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Returns the template the composition was built against.
    #[must_use]
    pub fn template_id(&self) -> &str {
        &self.template_id
    }

    /// Returns the generation of the template.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }
}

impl WebTemplateIndex {
    /// Builds a canonical composition from `values`.
    ///
    /// Every value is keyed by its node's place in the template, the
    /// composition is built through the Simplified Formats builder of
    /// `openehr-its`, and the result is validated against the same template
    /// before it is returned, so a composition this function answers with is
    /// one the CDR's own validation admits. `now` is the caller's current
    /// timestamp, which the builder uses for the `ctx/time` default alone
    /// (Simplified Formats, §Context); this crate reads no clock.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::UnknownNode`] when a value names a node of another
    /// template, [`PathError::OccurrenceCount`] when its occurrences do not
    /// match the repeating nodes above it, [`PathError::CompositionBuild`]
    /// when the builder refuses the values, and
    /// [`PathError::InvalidComposition`] when the result does not validate.
    pub fn build_composition(
        &self,
        values: &[NodeValue],
        now: &str,
    ) -> Result<CanonicalComposition, PathError> {
        let mut flat = serde_json::Map::new();
        for value in values {
            flat.insert(self.flat_key(value)?, value.value.clone());
        }
        let document = openehr_its::flat::sim::flat::parse_flat(&flat).map_err(|source| {
            PathError::CompositionBuild {
                template_id: self.template_id().to_owned(),
                source: Box::new(source),
            }
        })?;
        let built =
            openehr_its::flat::build::build_composition(&document, self.web_template(), now)
                .map_err(|source| PathError::CompositionBuild {
                    template_id: self.template_id().to_owned(),
                    source: Box::new(source),
                })?;
        let messages = openehr_its::rm_instance::validate_composition(&built, self.web_template());
        if !messages.is_empty() {
            return Err(PathError::InvalidComposition {
                template_id: self.template_id().to_owned(),
                messages: messages
                    .iter()
                    .map(|message| format!("{}: {}", message.path, message.message))
                    .collect(),
            });
        }
        Ok(CanonicalComposition {
            value: built,
            template_id: self.template_id().to_owned(),
            generation: self.generation(),
        })
    }

    /// Builds a canonical composition from a FLAT document.
    ///
    /// FLAT (simSDT) is the single-level serialization whose keys are template
    /// paths (Simplified Formats, §Flat format), the second form a composition
    /// is exchanged in. `now` is the caller's current timestamp, used for the
    /// `ctx/time` default alone; this crate reads no clock.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::CompositionBuild`] when the reader or the builder
    /// refuses the document and [`PathError::InvalidComposition`] when the
    /// result does not validate against the template.
    pub fn build_from_flat(
        &self,
        flat: &serde_json::Map<String, Value>,
        now: &str,
    ) -> Result<CanonicalComposition, PathError> {
        let document = openehr_its::flat::sim::flat::parse_flat(flat).map_err(|source| {
            PathError::CompositionBuild {
                template_id: self.template_id().to_owned(),
                source: Box::new(source),
            }
        })?;
        let built =
            openehr_its::flat::build::build_composition(&document, self.web_template(), now)
                .map_err(|source| PathError::CompositionBuild {
                    template_id: self.template_id().to_owned(),
                    source: Box::new(source),
                })?;
        let messages = openehr_its::rm_instance::validate_composition(&built, self.web_template());
        if !messages.is_empty() {
            return Err(PathError::InvalidComposition {
                template_id: self.template_id().to_owned(),
                messages: messages
                    .iter()
                    .map(|message| format!("{}: {}", message.path, message.message))
                    .collect(),
            });
        }
        Ok(CanonicalComposition {
            value: built,
            template_id: self.template_id().to_owned(),
            generation: self.generation(),
        })
    }

    /// Wraps canonical JSON a caller handed in, validated against this
    /// template.
    ///
    /// A composition that reaches the bridge from outside is validated before
    /// anything reads it, so a document of another template is a refusal
    /// rather than a read that answers nothing.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::InvalidComposition`] when the document does not
    /// validate against the template.
    pub fn accept(&self, value: Value) -> Result<CanonicalComposition, PathError> {
        let messages = openehr_its::rm_instance::validate_composition(&value, self.web_template());
        if !messages.is_empty() {
            return Err(PathError::InvalidComposition {
                template_id: self.template_id().to_owned(),
                messages: messages
                    .iter()
                    .map(|message| format!("{}: {}", message.path, message.message))
                    .collect(),
            });
        }
        Ok(CanonicalComposition {
            value,
            template_id: self.template_id().to_owned(),
            generation: self.generation(),
        })
    }

    /// Returns `composition` as a FLAT document.
    ///
    /// The template drives the walk, so the result is the same content in the
    /// other serialization (Simplified Formats,
    /// §Conversion Between Formats).
    ///
    /// # Errors
    ///
    /// Returns [`PathError::TemplateMismatch`] when the composition was built
    /// against another template and [`PathError::CompositionFlatten`] when the
    /// flattener refuses it.
    pub fn flatten(
        &self,
        composition: &CanonicalComposition,
    ) -> Result<serde_json::Map<String, Value>, PathError> {
        if composition.template_id != self.template_id() {
            return Err(PathError::TemplateMismatch {
                expected: self.template_id().to_owned(),
                found: composition.template_id.clone(),
            });
        }
        let document = openehr_its::flat::flatten::flatten_composition(
            &composition.value,
            self.web_template(),
        )
        .map_err(|source| PathError::CompositionFlatten {
            template_id: self.template_id().to_owned(),
            source: Box::new(source),
        })?;
        Ok(openehr_its::flat::sim::flat::emit_flat(&document))
    }

    /// Reads the value of one node out of a composition.
    ///
    /// The occurrences name the instance of each repeating node on the way to
    /// the target, outermost first, and become the positional predicates of
    /// the path the canonical JSON is navigated with.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::TemplateMismatch`] when the composition was built
    /// against another template, [`PathError::UnknownNode`] when the node
    /// belongs to another template, and [`PathError::OccurrenceCount`] when
    /// the occurrences do not match the repeating nodes above it.
    pub fn read(
        &self,
        composition: &CanonicalComposition,
        node: &ResolvedNode,
        occurrences: &[RmPosition],
    ) -> Result<Option<Value>, PathError> {
        if composition.template_id != self.template_id() {
            return Err(PathError::TemplateMismatch {
                expected: self.template_id().to_owned(),
                found: composition.template_id.clone(),
            });
        }
        let path = self.instance_path(node, occurrences)?;
        if path.segments.is_empty() {
            return Ok(Some(composition.value.clone()));
        }
        // NOTE: no specification governs this: our own design, an absent
        // optional node is a legitimate absence and never a defect, so it
        // reads as `None` rather than as an error.
        Ok(openehr_rm::v1_2::paths::item_at_path(&composition.value, &path).cloned())
    }

    /// Returns the FLAT key one value is written under.
    fn flat_key(&self, value: &NodeValue) -> Result<String, PathError> {
        let chain = self.chain(&value.flat_id)?;
        let repeating = chain.iter().filter(|entry| entry.node().repeats()).count();
        if repeating != value.occurrences.len() {
            return Err(PathError::OccurrenceCount {
                flat_id: value.flat_id.as_str().to_owned(),
                expected: repeating,
                given: value.occurrences.len(),
            });
        }
        let mut positions = value.occurrences.iter();
        let mut key = String::new();
        for entry in chain {
            if !key.is_empty() {
                key.push('/');
            }
            key.push_str(entry.id());
            if entry.node().repeats()
                && let Some(&position) = positions.next()
            {
                // NOTE: the FLAT instance index is 0-based (Simplified
                // Formats, §Instance Indexing), so the position converts.
                key.push(':');
                key.push_str(&FlatIndex::from(position).get().to_string());
            }
        }
        for segment in &value.sub_path {
            key.push('/');
            key.push_str(segment);
        }
        if let Some(ref datum) = value.datum {
            key.push('|');
            key.push_str(datum);
        }
        Ok(key)
    }

    /// Returns the path of one instance of `node`, with its positions set.
    fn instance_path(
        &self,
        node: &ResolvedNode,
        occurrences: &[RmPosition],
    ) -> Result<RmPath, PathError> {
        let chain = self.chain(node.flat_id())?;
        let repeating: Vec<usize> = chain
            .iter()
            .filter(|entry| entry.node().repeats())
            .filter_map(|entry| position_segment(entry.path()))
            .collect();
        if repeating.len() != occurrences.len() {
            return Err(PathError::OccurrenceCount {
                flat_id: node.flat_id().as_str().to_owned(),
                expected: repeating.len(),
                given: occurrences.len(),
            });
        }
        let mut path = chain
            .last()
            .map(|entry| entry.path().clone())
            .ok_or_else(|| PathError::UnknownNode {
                template_id: self.template_id().to_owned(),
                flat_id: node.flat_id().as_str().to_owned(),
            })?;
        for (&segment, &position) in repeating.iter().zip(occurrences.iter()) {
            if let Some(target) = path.segments.get_mut(segment) {
                target.predicate.position = usize::try_from(position.get()).ok();
            }
        }
        Ok(path)
    }
}

/// Returns the segment of `path` a repeat of its node applies to.
///
/// The Web Template compacts the RM levels an archetype leaves unconstrained
/// (Simplified Formats, §Level Removal), so a node's own path may end below
/// the container that repeats. The repeat applies to the deepest segment
/// carrying an archetype node id, which is that container. No specification
/// governs this mapping: it is FerroBRIDGE's own rule.
fn position_segment(path: &RmPath) -> Option<usize> {
    path.segments
        .iter()
        .rposition(|segment| segment.predicate.archetype_node_id.is_some())
        .or_else(|| path.segments.len().checked_sub(1))
}

#[cfg(test)]
mod tests {
    use super::FlatIndex;
    use super::PositionError;
    use super::RmPosition;

    #[test]
    fn the_two_index_bases_differ_by_one() {
        let first = RmPosition::first();
        assert_eq!(first.get(), 1);
        assert_eq!(FlatIndex::from(first).get(), 0);
        let third = RmPosition::new(3).expect("3 is a position");
        assert_eq!(FlatIndex::from(third).get(), 2);
        assert_eq!(
            RmPosition::try_from(FlatIndex::new(2)).expect("2 is an index"),
            third
        );
    }

    #[test]
    fn a_zero_position_and_the_last_index_are_refused() {
        assert_eq!(RmPosition::new(0), Err(PositionError::Zero));
        assert_eq!(
            RmPosition::try_from(FlatIndex::new(u32::MAX)),
            Err(PositionError::Overflow)
        );
    }
}
