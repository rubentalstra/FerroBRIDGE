// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! What a run produced, and the declared set of losses it took.
//!
//! An element the program cannot map refuses the unit
//! (`docs/architecture.md` §9), so a warning here is never "something went
//! wrong and the run carried on". Each variant of [`Warning`] is a loss the
//! specification itself declares: a mapping the direction skips, a
//! composition field the engine defaulted, the occurrences a `0..n` into a
//! `0..1` dropped, a one-way row a data-type cell took, and a reference whose
//! resolution belongs to the facade. Nothing else is a warning, and the
//! round-trip tests assert the set exactly.

use core::fmt;

use crate::engine::lens::Fallback;

/// A produced document with the losses the run declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome<T> {
    value: T,
    warnings: Vec<Warning>,
}

impl<T> Outcome<T> {
    /// Wraps a produced value that took no declared loss.
    #[must_use]
    pub const fn clean(value: T) -> Self {
        Self {
            value,
            warnings: Vec::new(),
        }
    }

    /// Wraps a produced value with the losses the run declared.
    #[must_use]
    pub const fn new(value: T, warnings: Vec<Warning>) -> Self {
        Self { value, warnings }
    }

    /// Returns the produced value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns the produced value, consuming the outcome.
    #[must_use]
    pub fn into_value(self) -> T {
        self.value
    }

    /// Returns the losses the run declared, in the order it took them.
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Records one loss.
    pub fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Returns this outcome with `value` in place of the produced one.
    #[must_use]
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> Outcome<U> {
        Outcome {
            value: transform(self.value),
            warnings: self.warnings,
        }
    }
}

/// One loss a run declared.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Warning {
    /// A mapping the direction does not run.
    Skipped {
        /// The dotted name of the mapping inside its model mapping.
        mapping: String,
        /// Why the mapping did not run.
        reason: SkipReason,
    },
    /// A composition field the engine filled because no mapping did.
    ///
    /// The defaults are the ones `engine/defaults-for-fields.adoc` names, and
    /// they apply going into openEHR only.
    Defaulted {
        /// The openEHR path of the field.
        field: String,
    },
    /// The occurrences a `0..n` input lost writing into a `0..1` output.
    ///
    /// "If we transform from a `0..n` to a `0..1` field, only the last entry
    /// of `0..n` is mapped" (`recurrence/TransformingList.adoc`).
    LastOfMany {
        /// The output path that took the last occurrence.
        path: String,
        /// How many occurrences the write dropped.
        dropped: usize,
    },
    /// A data-type row the specification writes for one direction only.
    OneWayFallback {
        /// The cell that took the row.
        cell: String,
        /// The attribute the row filled.
        field: String,
    },
    /// A reference whose resolution belongs to the facade.
    ///
    /// `PARTY_IDENTIFIED.adoc` resolves a `Reference` through the demographics
    /// server and puts the resolved resource's identifier in `identifiers`.
    /// The engine makes no call, so it records the reference instead of
    /// inventing an identifier for it.
    DeferredReference {
        /// The reference string the facade resolves.
        reference: String,
    },
}

impl Warning {
    /// Records the one-way row a data-type cell took.
    #[must_use]
    pub fn fallback(taken: &Fallback) -> Self {
        Self::OneWayFallback {
            cell: String::from(taken.cell()),
            field: String::from(taken.field()),
        }
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Skipped {
                ref mapping,
                ref reason,
            } => write!(f, "the mapping {mapping} did not run: {reason}"),
            Self::Defaulted { ref field } => write!(f, "the engine filled {field}"),
            Self::LastOfMany { ref path, dropped } => write!(
                f,
                "{path} holds one value, so the write dropped {dropped} earlier occurrences"
            ),
            Self::OneWayFallback {
                ref cell,
                ref field,
            } => write!(f, "{cell} filled {field} by a rule that runs one way"),
            Self::DeferredReference { ref reference } => {
                write!(f, "{reference} is left for the facade to resolve")
            }
        }
    }
}

/// Why a mapping did not run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipReason {
    /// The mapping's `unidirectional` names the other direction.
    Unidirectional,
    /// A preprocessor gate of the slotted file did not admit the input.
    PreprocessorGate {
        /// The `metadata.name` of the slotted file whose gate closed.
        model: String,
    },
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unidirectional => {
                f.write_str("its unidirectional marker names the other direction")
            }
            Self::PreprocessorGate { ref model } => {
                write!(
                    f,
                    "the preprocessor gate of {model} did not admit the input"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Outcome;
    use super::SkipReason;
    use super::Warning;
    use crate::engine::lens::Fallback;

    #[test]
    fn a_clean_outcome_declares_no_loss() {
        let outcome = Outcome::clean(7);
        assert_eq!(*outcome.value(), 7);
        assert!(
            outcome.warnings().is_empty(),
            "a clean run declares no loss"
        );
    }

    #[test]
    fn the_losses_keep_the_order_the_run_took_them() {
        let mut outcome = Outcome::clean(String::from("document"));
        outcome.warn(Warning::Skipped {
            mapping: String::from("bodySite"),
            reason: SkipReason::Unidirectional,
        });
        outcome.warn(Warning::fallback(&Fallback::new(
            "DV_CODED_TEXT against CodeableConcept",
            "DV_TEXT.value",
        )));
        assert_eq!(outcome.warnings().len(), 2);
        assert_eq!(
            outcome.warnings().first().map(ToString::to_string),
            Some(String::from(
                "the mapping bodySite did not run: its unidirectional marker names the other direction"
            ))
        );
    }

    #[test]
    fn a_dropped_occurrence_names_its_path_and_its_count() {
        let warning = Warning::LastOfMany {
            path: String::from("$archetype/data[at0001]/items[at0002]"),
            dropped: 2,
        };
        assert!(
            warning
                .to_string()
                .contains("dropped 2 earlier occurrences"),
            "the warning states how much was lost: {warning}"
        );
    }
}
