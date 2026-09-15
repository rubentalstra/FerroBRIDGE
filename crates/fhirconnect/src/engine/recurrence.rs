// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Where a mapping writes when the two sides repeat differently.
//!
//! `docs/specs/fhirconnect/modules/ROOT/pages/recurrence/` names three rules
//! and the engine follows all three here, once, for both sides:
//!
//! - a later mapping to the same `0..1` path overwrites the earlier one
//!   (`Overwriting.adoc`), which follows from every such mapping writing at
//!   the same place;
//! - "when transforming an element that is `0..n`, FHIRconnect always appends"
//!   (`TransformingList.adoc`), so each input occurrence takes the next index
//!   after whatever the output already holds;
//! - "if we transform from a `0..n` to a `0..1` field, only the last entry of
//!   `0..n` is mapped" (`TransformingList.adoc`), and the occurrences that did
//!   not survive are a declared loss rather than silence.
//!
//! An occurrence is a structured index, one entry per repeating element on the
//! way to the target, never a pattern over a rendered path. A child mapping
//! writes under the occurrence its parent is bound to, which is what makes
//! `PopulatingAnEntry.adoc`'s correct shape iterate the two lists against each
//! other and its counter-example append a sibling per child mapping.

use openehr_mapping_core::composition::FlatIndex;
use openehr_mapping_core::composition::PositionError;
use openehr_mapping_core::composition::RmPosition;

use crate::tree::Occurrence;

/// How many values the element a mapping writes to may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cardinality {
    /// The element holds one value, so a second write overwrites the first.
    One,
    /// The element repeats, so each write appends.
    Many,
}

impl Cardinality {
    /// Returns the cardinality of an element that repeats or does not.
    #[must_use]
    pub const fn of(repeats: bool) -> Self {
        if repeats { Self::Many } else { Self::One }
    }
}

/// Where each occurrence of the input writes, and what the write dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    slots: Vec<Option<usize>>,
    dropped: usize,
}

impl Placement {
    /// Decides where `inputs` occurrences write into an element of
    /// `cardinality` that already holds `already` values.
    ///
    /// A repeating element takes one index per input occurrence, counting on
    /// from what it holds. An element that holds one value takes the last
    /// input occurrence and drops the rest.
    #[must_use]
    pub fn decide(inputs: usize, cardinality: Cardinality, already: usize) -> Self {
        match cardinality {
            Cardinality::Many => Self {
                slots: (0..inputs)
                    .map(|offset| Some(already.saturating_add(offset)))
                    .collect(),
                dropped: 0,
            },
            Cardinality::One => {
                let last = inputs.saturating_sub(1);
                Self {
                    slots: (0..inputs)
                        .map(|offset| (offset == last).then_some(0))
                        .collect(),
                    dropped: last,
                }
            }
        }
    }

    /// Returns the index each input occurrence writes at, `None` for one the
    /// write dropped.
    #[must_use]
    pub fn slots(&self) -> &[Option<usize>] {
        &self.slots
    }

    /// Returns how many input occurrences the write dropped.
    #[must_use]
    pub const fn dropped(&self) -> usize {
        self.dropped
    }

    /// Returns whether the write dropped any occurrence.
    #[must_use]
    pub const fn lossy(&self) -> bool {
        self.dropped > 0
    }
}

/// Returns the FHIR occurrence a write takes under `parent`.
///
/// `parent` is the occurrence the enclosing mapping is bound to, and `index`
/// is what [`Placement`] decided for the repeating element below it. An output
/// path with no repeating element below the parent takes the parent's
/// occurrence unchanged.
#[must_use]
pub fn under(parent: &Occurrence, index: Option<usize>) -> Occurrence {
    let mut indices: Vec<usize> = parent.indices().to_vec();
    if let Some(index) = index {
        indices.push(index);
    }
    Occurrence::new(indices)
}

/// Returns the openEHR occurrence a write takes under `parent`.
///
/// An openEHR positional predicate is 1-based (openEHR BASE Release 1.2.0
/// §Paths and Locators) and a [`Placement`] index is 0-based, so the two never
/// meet as bare integers: the conversion goes through [`FlatIndex`].
///
/// # Errors
///
/// Returns [`PositionError::Overflow`] for an index with no 1-based position.
pub fn positions_under(
    parent: &[RmPosition],
    index: Option<usize>,
) -> Result<Vec<RmPosition>, PositionError> {
    let mut positions: Vec<RmPosition> = parent.to_vec();
    if let Some(index) = index {
        let index = u32::try_from(index).map_err(|_refused| PositionError::Overflow)?;
        positions.push(RmPosition::try_from(FlatIndex::new(index))?);
    }
    Ok(positions)
}

#[cfg(test)]
mod tests {
    use super::Cardinality;
    use super::Placement;
    use super::positions_under;
    use super::under;
    use crate::tree::Occurrence;
    use openehr_mapping_core::composition::RmPosition;

    /// Returns the occurrences `inputs` input occurrences write at, under the
    /// parent occurrence `parent`.
    fn writes(parent: &[usize], inputs: usize, cardinality: Cardinality) -> Vec<String> {
        let parent = Occurrence::new(parent.to_vec());
        Placement::decide(inputs, cardinality, 0)
            .slots()
            .iter()
            .filter_map(|slot| slot.map(|index| under(&parent, Some(index)).to_string()))
            .collect()
    }

    #[test]
    fn a_later_write_to_one_value_takes_the_same_place() {
        // Overwriting.adoc: two mappings address the same 0..1 openEHR path,
        // and "it will overwrite the first one, since it adresses the same
        // path".
        let first = Placement::decide(1, Cardinality::One, 0);
        let second = Placement::decide(1, Cardinality::One, 1);
        assert_eq!(first.slots(), second.slots(), "both writes take one place");
        assert_eq!(first.slots(), [Some(0)]);
    }

    #[test]
    fn two_mappings_into_one_list_append() {
        // TransformingList.adoc: bodySite and the anatomical-location cluster
        // both map into the 0..n FHIR bodySite, so "both entries would be
        // appended into the bodySite".
        let first = Placement::decide(1, Cardinality::Many, 0);
        let second = Placement::decide(1, Cardinality::Many, 1);
        assert_eq!(first.slots(), [Some(0)], "the first write starts the list");
        assert_eq!(second.slots(), [Some(1)], "the second write appends");
        assert!(!first.lossy(), "an append loses nothing");
    }

    #[test]
    fn many_into_one_takes_the_last_and_records_the_rest() {
        // TransformingList.adoc: "if we transform from a 0..n to a 0..1 field,
        // only the last entry of 0..n is mapped".
        let placement = Placement::decide(3, Cardinality::One, 0);
        assert_eq!(placement.slots(), [None, None, Some(0)]);
        assert_eq!(placement.dropped(), 2, "the two earlier entries are a loss");
        assert!(placement.lossy(), "the loss is declared");
    }

    #[test]
    fn the_iterated_shape_pairs_the_two_lists() {
        // PopulatingAnEntry.adoc: `collection` and `data[at0001]` are both
        // 0..n and sit at the same level, so each child mapping writes under
        // the occurrence its parent is bound to.
        assert_eq!(writes(&[0], 1, Cardinality::Many), ["0.0"]);
        assert_eq!(writes(&[1], 1, Cardinality::Many), ["1.0"]);
    }

    #[test]
    fn the_unpaired_shape_appends_a_sibling_per_child_mapping() {
        // PopulatingAnEntry.adoc §Wrong mapping: with `data[at0001]` inside the
        // child path instead of the iterated parent, "FHIRconnect always
        // appends to a 0..n path if it's not iterated", so two child mappings
        // over two parent occurrences make four entries rather than two.
        let mut appended = Vec::new();
        let mut already = 0usize;
        for _parent in 0..2 {
            for _child in 0..2 {
                let placement = Placement::decide(1, Cardinality::Many, already);
                for slot in placement.slots().iter().flatten() {
                    appended.push(*slot);
                    already = already.saturating_add(1);
                }
            }
        }
        assert_eq!(
            appended,
            [0, 1, 2, 3],
            "each child mapping of each parent occurrence appends its own entry"
        );
    }

    #[test]
    fn the_double_nesting_counter_example_flattens_the_inner_list() {
        // PopulatingAnEntry.adoc §Example of double nesting: the correct shape
        // pairs category with at0094 and text with at0063, giving two outer
        // entries of two inner ones; the counter-example writes one outer entry
        // per inner value, giving four.
        let paired: Vec<String> = (0..2)
            .flat_map(|outer| writes(&[outer], 2, Cardinality::Many))
            .collect();
        assert_eq!(paired, ["0.0", "0.1", "1.0", "1.1"], "two entries of two");
        let flattened: Vec<String> = writes(&[], 4, Cardinality::Many);
        assert_eq!(flattened, ["0", "1", "2", "3"], "four entries of one");
    }

    #[test]
    fn an_openehr_position_counts_from_one() {
        let positions = positions_under(&[], Some(0)).expect("0 has a position");
        assert_eq!(
            positions.first().copied().map(RmPosition::get),
            Some(1),
            "the first instance is [1]"
        );
        let deeper = positions_under(&[RmPosition::first()], Some(2)).expect("2 has a position");
        assert_eq!(
            deeper
                .iter()
                .map(|position| position.get())
                .collect::<Vec<u32>>(),
            [1, 3],
            "the parent position leads and the index follows"
        );
    }

    #[test]
    fn a_write_with_no_repeating_element_keeps_its_parent_occurrence() {
        let parent = Occurrence::new([2, 1]);
        assert_eq!(under(&parent, None), parent, "nothing is appended");
        assert_eq!(under(&parent, Some(0)).to_string(), "2.1.0");
    }
}
