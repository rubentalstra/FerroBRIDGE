// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Test support for the FerroBRIDGE suites, consumed as a path-only
//! dev-dependency so `cargo package` strips it.
//!
//! [`matrix_pin`] reads the pin matrix, so a crate's version constant can be
//! asserted against the single source of truth; [`conformance`] records the
//! corpus verdicts the conformance gate reads; [`containers`] starts the
//! PostgreSQL and the reference CDR the end-to-end lane runs against;
//! [`fixtures`] holds the synthetic documents the suites commit and map;
//! [`laws`] compares the two ends of a round trip;
//! [`stubs`] carries the upstream response shapes for `wiremock`; and
//! [`vocabulary`] holds the synthetic OHDSI vocabulary and its loader.
#![doc(test(attr(deny(warnings))))]

// TODO(#88): the synthetic vocabulary takes the observed Athena export shape
// once it is recorded, so the production loader is what the tests exercise.

pub mod conformance;
pub mod containers;
pub mod fixtures;
pub mod laws;
pub mod stubs;
pub mod vocabulary;

use std::fmt;
use std::path::PathBuf;

/// The pin matrix at `docs/VERSIONS.md`, relative to this crate's manifest.
const MATRIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/VERSIONS.md");

/// A pin could not be read from the matrix.
#[derive(Debug)]
pub enum PinError {
    /// The matrix file could not be read.
    Read {
        /// The path that was tried.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// No table row has the requested item in its first cell.
    Missing {
        /// The item that was looked up.
        item: String,
    },
}

impl fmt::Display for PinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, .. } => {
                write!(f, "cannot read the pin matrix at {}", path.display())
            }
            Self::Missing { item } => write!(f, "the pin matrix has no row for {item}"),
        }
    }
}

impl std::error::Error for PinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Missing { .. } => None,
        }
    }
}

/// Returns the first token of the `Pin` cell of the matrix row whose first
/// cell is `item`, with backticks removed.
///
/// This mirrors what `scripts/checks/versions.sh` reads, so a crate constant
/// and the guard agree on the same value.
///
/// # Errors
///
/// Returns [`PinError::Read`] when `docs/VERSIONS.md` cannot be read and
/// [`PinError::Missing`] when no row carries `item`.
pub fn matrix_pin(item: &str) -> Result<String, PinError> {
    let path = PathBuf::from(MATRIX);
    let text = std::fs::read_to_string(&path).map_err(|source| PinError::Read {
        path: path.clone(),
        source,
    })?;
    text.lines()
        .find_map(|line| {
            let mut cells = line
                .split('|')
                .skip(1)
                .map(|c| c.replace('`', "").trim().to_owned());
            let key = cells.next()?;
            let pin = cells.next()?;
            (key == item).then(|| pin.split_whitespace().next().unwrap_or_default().to_owned())
        })
        .ok_or_else(|| PinError::Missing {
            item: item.to_owned(),
        })
}
