// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A v2 value at some depth of a field.

use crate::parse::{Component, Repetition};

/// A v2 value at some depth of a field.
#[derive(Debug, Clone, Copy)]
pub(super) enum Datum<'m> {
    /// A whole segment.
    Segment,
    /// One repetition of a field.
    Repetition(&'m Repetition),
    /// One component.
    Component(&'m Component),
    /// One subcomponent's text.
    Text(&'m str),
    /// Nothing at that position.
    Absent,
}

impl<'m> Datum<'m> {
    /// The part at `position`, from 1, one level down.
    pub(super) fn child(self, position: usize) -> Self {
        match self {
            Self::Repetition(repetition) => repetition
                .component(position)
                .map_or(Self::Absent, Self::Component),
            Self::Component(component) => component
                .subcomponents()
                .get(position.saturating_sub(1))
                .map_or(Self::Absent, |text| Self::Text(text.as_str())),
            Self::Text(text) if position == 1 => Self::Text(text),
            Self::Segment | Self::Text(_) | Self::Absent => Self::Absent,
        }
    }

    /// The part a component path names.
    pub(super) fn at(self, path: &[usize]) -> Self {
        path.iter()
            .fold(self, |datum, position| datum.child(*position))
    }

    /// The first leaf's text, when it is valued.
    pub(super) fn text(self) -> Option<&'m str> {
        match self {
            Self::Repetition(repetition) => repetition.text(),
            Self::Component(component) => component.text(),
            Self::Text(text) => Some(text).filter(|text| crate::parse::valued(text)),
            Self::Segment | Self::Absent => None,
        }
    }

    /// Whether any leaf holds a value.
    pub(super) fn valued(self) -> bool {
        match self {
            Self::Segment => true,
            Self::Repetition(repetition) => repetition.is_valued(),
            Self::Component(component) => component.is_valued(),
            Self::Text(text) => crate::parse::valued(text),
            Self::Absent => false,
        }
    }

    /// Whether a leaf after the first holds a value.
    pub(super) fn extra(self) -> bool {
        match self {
            Self::Repetition(repetition) => {
                repetition
                    .components()
                    .iter()
                    .skip(1)
                    .any(Component::is_valued)
                    || repetition
                        .component(1)
                        .is_some_and(|first| Datum::Component(first).extra())
            }
            Self::Component(component) => component
                .subcomponents()
                .iter()
                .skip(1)
                .any(|text| crate::parse::valued(text)),
            Self::Segment | Self::Text(_) | Self::Absent => false,
        }
    }

    /// The positions of the valued parts one level down.
    pub(super) fn valued_parts(self) -> Vec<usize> {
        let count = match self {
            Self::Repetition(repetition) => repetition.components().len(),
            Self::Component(component) => component.subcomponents().len(),
            Self::Segment | Self::Text(_) | Self::Absent => 0,
        };
        (1..=count)
            .filter(|position| self.child(*position).valued())
            .collect()
    }
}
