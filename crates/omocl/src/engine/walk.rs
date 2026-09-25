// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The first pass of a run: every record instance of the composition, with
//! the value each of its columns chose.
//!
//! Three rules fill silences of OMOCL here, each our own design
//! (<https://github.com/SevKohler/OMOCL> states none of them). The
//! `alternatives` of a column are tried in order and the first present value
//! wins, the only reading the library is consistent with. A column the
//! mapping does not mark `optional` with no present alternative refuses the
//! record. A record iterates the instances of its scope root and of its
//! `base_path`, one row per instance, and a column path that matches several
//! nodes below one instance refuses the record.

use core::str::FromStr;
use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde_json::Value;

use crate::engine::RefusalKind;
use crate::engine::custom::CustomConverter;
use crate::engine::datum;
use crate::engine::datum::Datum;
use crate::engine::datum::DatumError;
use crate::engine::datum::Number;
use crate::engine::navigate::Instance;
use crate::engine::navigate::follow;
use crate::model::ast::AtCode;
use crate::model::projection::KeyProjection;
use crate::resolve::program::BoundAlternative;
use crate::resolve::program::BoundColumn;
use crate::resolve::program::BoundEntry;
use crate::resolve::program::BoundFactor;
use crate::resolve::program::BoundPath;
use crate::resolve::program::BoundRecord;
use crate::resolve::program::Hop;
use crate::resolve::program::Program;
use crate::resolve::program::Scope;

/// Why a record was refused, before it is rendered into the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused {
    pub(crate) column: Option<&'static str>,
    pub(crate) path: String,
    pub(crate) kind: RefusalKind,
    pub(crate) message: String,
}

impl Refused {
    /// Creates a refusal about one column at one instance path.
    pub(crate) fn new(
        kind: RefusalKind,
        column: Option<&'static str>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            column,
            path: path.into(),
            kind,
            message: message.into(),
        }
    }
}

/// One column's chosen value.
#[derive(Debug, Clone)]
pub(crate) struct Chosen {
    pub(crate) projection: &'static KeyProjection,
    pub(crate) datum: Datum,
    pub(crate) path: String,
}

/// One record instance, with the value each column chose.
#[derive(Debug)]
pub(crate) struct Draft<'p> {
    pub(crate) mapping: String,
    pub(crate) record: &'p BoundRecord,
    pub(crate) root_path: String,
    pub(crate) path: String,
    pub(crate) columns: Vec<Chosen>,
    pub(crate) groups: Vec<usize>,
}

/// A refused record instance, with where it came from.
#[derive(Debug)]
pub(crate) struct Failed<'p> {
    pub(crate) mapping: String,
    pub(crate) record: &'p BoundRecord,
    pub(crate) refused: Refused,
}

/// A `CustomMapping` to run over the records of one scope instance.
#[derive(Debug)]
pub(crate) struct Task<'p> {
    pub(crate) group: usize,
    pub(crate) converter: &'p dyn CustomConverter,
}

/// The first pass over one composition.
#[derive(Debug)]
pub(crate) struct Walk<'p, 'c> {
    composition: &'c Value,
    pub(crate) drafts: Vec<Draft<'p>>,
    pub(crate) failed: Vec<Failed<'p>>,
    pub(crate) tasks: Vec<Task<'p>>,
    pub(crate) consumed: BTreeSet<String>,
    pub(crate) scopes: Vec<(String, String)>,
    groups: usize,
}

impl<'p, 'c> Walk<'p, 'c> {
    /// Walks every scope of `program` over `composition`.
    pub(crate) fn run(program: &'p Program, composition: &'c Value) -> Self {
        let mut walk = Self {
            composition,
            drafts: Vec::new(),
            failed: Vec::new(),
            tasks: Vec::new(),
            consumed: BTreeSet::new(),
            scopes: Vec::new(),
            groups: 0,
        };
        let root = Instance::root(composition);
        for scope in program.roots() {
            let hop = Hop {
                up: 0,
                down: scope.root().segments.clone(),
            };
            for instance in follow(composition, &root, &hop) {
                walk.scope(scope, &instance, &[]);
            }
        }
        walk
    }

    /// Walks one instance of one scope.
    fn scope(&mut self, scope: &'p Scope, instance: &Instance<'c>, parents: &[usize]) {
        let group = self.groups;
        self.groups = self.groups.saturating_add(1);
        let mut chain = parents.to_vec();
        chain.push(group);
        self.scopes
            .push((instance.path(), scope.mapping().as_str().to_owned()));
        for entry in scope.entries() {
            match *entry {
                BoundEntry::Record(ref record) => {
                    let anchors = match record.base {
                        None => vec![instance.clone()],
                        Some(ref hop) => follow(self.composition, instance, hop),
                    };
                    for anchor in anchors {
                        self.record(scope, record, instance, &anchor, &chain);
                    }
                }
                BoundEntry::Include(ref include) => {
                    for inner in follow(self.composition, instance, &include.hop) {
                        for child in include.scopes() {
                            self.scope(child, &inner, &chain);
                        }
                    }
                }
                BoundEntry::Custom(ref custom) => self.tasks.push(Task {
                    group,
                    converter: custom.converter(),
                }),
            }
        }
    }

    /// Drafts one record instance, or records its refusal.
    fn record(
        &mut self,
        scope: &'p Scope,
        record: &'p BoundRecord,
        root: &Instance<'c>,
        anchor: &Instance<'c>,
        chain: &[usize],
    ) {
        let mapping = scope.mapping().as_str().to_owned();
        let mut columns = Vec::new();
        let mut consumed = Vec::new();
        for column in record.columns() {
            match self.choose(anchor, column, &mut consumed) {
                Ok(Some((datum, path))) => columns.push(Chosen {
                    projection: column.projection,
                    datum,
                    path,
                }),
                Ok(None) if column.optional => {}
                Ok(None) => {
                    let refused = Refused::new(
                        RefusalKind::MissingColumn,
                        Some(column.key()),
                        anchor.path(),
                        format!(
                            "no alternative of the required `{}` is present",
                            column.key()
                        ),
                    );
                    self.failed.push(Failed {
                        mapping,
                        record,
                        refused,
                    });
                    return;
                }
                Err(refused) => {
                    self.failed.push(Failed {
                        mapping,
                        record,
                        refused,
                    });
                    return;
                }
            }
        }
        self.consumed.extend(consumed);
        self.drafts.push(Draft {
            mapping,
            record,
            root_path: root.path(),
            path: anchor.path(),
            columns,
            groups: chain.to_vec(),
        });
    }

    /// Returns the value the first present alternative of `column` produces.
    fn choose(
        &self,
        anchor: &Instance<'c>,
        column: &BoundColumn,
        consumed: &mut Vec<String>,
    ) -> Result<Option<(Datum, String)>, Refused> {
        let key = column.key();
        for alternative in column.alternatives() {
            match *alternative {
                BoundAlternative::Unbound { .. } => {}
                BoundAlternative::Code(code) => {
                    return Ok(Some((Datum::Literal(code), anchor.path())));
                }
                BoundAlternative::Path(ref path) => {
                    if let Some((datum, at)) = self.read(anchor, path, key)? {
                        consumed.push(at.clone());
                        return Ok(Some((datum, at)));
                    }
                }
                BoundAlternative::ConceptMap {
                    ref path,
                    ref mapping,
                } => {
                    let Some((datum, at)) = self.read(anchor, path, key)? else {
                        continue;
                    };
                    let Datum::Coded { code, .. } = datum else {
                        return Err(Refused::new(
                            RefusalKind::UnsupportedValue,
                            Some(key),
                            at,
                            "a conceptMap reads a coded value",
                        ));
                    };
                    // NOTE: a code that is not an at-code is legitimately
                    // absent from the table, which lists at-codes only.
                    let code = code.code_string;
                    let concept = AtCode::from_str(&code)
                        .ok()
                        .and_then(|at_code| mapping.get(&at_code).copied());
                    let Some(concept) = concept else {
                        return Err(Refused::new(
                            RefusalKind::UnlistedAtCode,
                            Some(key),
                            at,
                            format!("the conceptMap of `{key}` does not list `{code}`"),
                        ));
                    };
                    consumed.push(at.clone());
                    return Ok(Some((Datum::Mapped { concept, code }, at)));
                }
                BoundAlternative::Multiplication(ref factors) => {
                    if let Some(product) = self.product(anchor, factors, key, consumed)? {
                        return Ok(Some((Datum::Number(product), anchor.path())));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Returns the product of `factors`, `None` when one is absent.
    ///
    /// The product is exact decimal arithmetic and a result outside the
    /// decimal range refuses the record. OMOCL states no arithmetic for
    /// `multiplication`, so no specification governs this: our own design.
    fn product(
        &self,
        anchor: &Instance<'c>,
        factors: &[BoundFactor],
        key: &'static str,
        consumed: &mut Vec<String>,
    ) -> Result<Option<Number>, Refused> {
        let mut product = Decimal::ONE;
        let mut read = Vec::new();
        for factor in factors {
            let value = match *factor {
                BoundFactor::Code(code) => Decimal::from(code),
                BoundFactor::Path(ref path) => {
                    let Some((datum, at)) = self.read(anchor, path, key)? else {
                        return Ok(None);
                    };
                    let value = match datum {
                        Datum::Quantity { ref magnitude, .. } => magnitude.value,
                        Datum::Number(ref number) => number.value,
                        _ => {
                            return Err(Refused::new(
                                RefusalKind::UnsupportedValue,
                                Some(key),
                                at,
                                "a multiplication factor is a number",
                            ));
                        }
                    };
                    read.push(at);
                    value
                }
            };
            product = product.checked_mul(value).ok_or_else(|| {
                Refused::new(
                    RefusalKind::Overflow,
                    Some(key),
                    anchor.path(),
                    format!("the multiplication of `{key}` overflows the decimal range"),
                )
            })?;
        }
        consumed.append(&mut read);
        Ok(Some(Number {
            value: product,
            text: product.normalize().to_string(),
        }))
    }

    /// Reads the one node `path` names below `anchor`.
    fn read(
        &self,
        anchor: &Instance<'c>,
        path: &BoundPath,
        key: &'static str,
    ) -> Result<Option<(Datum, String)>, Refused> {
        let found = follow(self.composition, anchor, path.hop());
        match found.as_slice() {
            [] => Ok(None),
            [only] => {
                let at = only.path();
                match datum::read(only.value()) {
                    Ok(Some(datum)) => Ok(Some((datum, at))),
                    Ok(None) => Ok(None),
                    Err(error) => {
                        let kind = match error {
                            DatumError::Unsupported { .. } => RefusalKind::UnsupportedValue,
                            DatumError::Malformed { .. } => RefusalKind::InvalidValue,
                        };
                        Err(Refused::new(kind, Some(key), at, error.to_string()))
                    }
                }
            }
            several => Err(Refused::new(
                RefusalKind::Multiple,
                Some(key),
                anchor.path(),
                format!(
                    "`{}` matches {} nodes and the record does not iterate them",
                    path.written(),
                    several.len()
                ),
            )),
        }
    }
}
