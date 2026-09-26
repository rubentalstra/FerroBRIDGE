// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data type codes a legacy version's fields name, with the base data
//! type each version-specific code stands for.
//!
//! `datatypes.json` lists every code a version's fields name, `CM_MSG` and
//! `CE_0051` among them, with a name and no column naming a base type. The
//! base is derived from the code ([`base_of`]): `CM_<name>` is the composite
//! `<name>`, `<type>_<table>` is `<type>` bound to that four-digit table, and
//! `TS` is `DTM`. A derived base must be a code the version's own table or
//! the v2.9.1 definitions define, and a code that resolves to none carries
//! none.

use std::collections::{BTreeMap, BTreeSet};

use crate::v2::legacy::lower::LegacyError;
use crate::v2::legacy::source::VersionTables;
use crate::v2::lower::{DataType, Segment};

/// One data type code of a version's tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyDataType {
    /// The code, for example `CE_0051`.
    pub code: String,
    /// The name `datatypes.json` gives it.
    pub name: String,
    /// The base data type the code stands for, when it stands for one.
    pub base: Option<Base>,
}

/// The base data type of a version-specific code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// The base code, for example `CE`.
    pub code: String,
    /// The table the code binds the base to, for example `0051`.
    pub table: Option<String>,
}

/// Returns the base `code` stands for, given the codes the version's own
/// table defines (`own`) and the v2.9.1 data types (`current`).
///
/// No specification governs this: our own design, reading the export's
/// codes; `datatypes.json` names `CE_0051` `CE_NORM mit Tab. 0051` and
/// `CM_MSG` `Message Type`, the name of the v2.9.1 `MSG`.
#[must_use]
pub fn base_of(
    code: &str,
    own: &BTreeSet<&str>,
    current: &BTreeMap<String, DataType>,
) -> Option<Base> {
    let defined = |base: &str| base != code && (own.contains(base) || current.contains_key(base));
    let base = |code: &str, table: Option<&str>| Base {
        code: code.to_owned(),
        table: table.map(str::to_owned),
    };
    // NOTE: no specification governs this: our own design; `TS` is the time
    // stamp whose first component is the `DTM` the v2.9.1 fields name instead.
    if code == "TS" {
        return defined("DTM").then(|| base("DTM", None));
    }
    if let Some(name) = code.strip_prefix("CM_") {
        return defined(name).then(|| base(name, None));
    }
    let (name, table) = code.rsplit_once('_')?;
    let four_digits = table.len() == 4 && table.bytes().all(|b| b.is_ascii_digit());
    (four_digits && defined(name)).then(|| base(name, Some(table)))
}

/// Lowers every data type code the fields of `segments` name, from the
/// version's `datatypes.json`.
///
/// # Errors
///
/// Returns [`LegacyError::Invalid`] when a field names a code the version's
/// `datatypes.json` has no row for, or the file lists a code twice.
pub fn lower_data_types(
    version: &VersionTables,
    segments: &BTreeMap<String, Segment>,
    current: &BTreeMap<String, DataType>,
) -> Result<BTreeMap<String, LegacyDataType>, LegacyError> {
    let file = version.file("datatypes");
    let mut rows = BTreeMap::new();
    for row in &version.data_types {
        if rows.insert(row.id.as_str(), row).is_some() {
            return Err(LegacyError::Invalid {
                file,
                row: row.id.clone(),
                reason: String::from("the code is listed twice"),
            });
        }
    }
    let own: BTreeSet<&str> = rows.keys().copied().collect();
    let mut lowered = BTreeMap::new();
    for segment in segments.values() {
        for field in &segment.fields {
            let Some(code) = field.data_type.as_deref() else {
                continue;
            };
            if lowered.contains_key(code) {
                continue;
            }
            let Some(row) = rows.get(code) else {
                return Err(LegacyError::Invalid {
                    file,
                    row: field.id.clone(),
                    reason: format!("the data type {code} has no row"),
                });
            };
            lowered.insert(
                code.to_owned(),
                LegacyDataType {
                    code: code.to_owned(),
                    name: row.description.clone(),
                    base: base_of(code, &own, current),
                },
            );
        }
    }
    Ok(lowered)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{Base, base_of};
    use crate::v2::lower::DataType;

    fn current(codes: &[&str]) -> BTreeMap<String, DataType> {
        codes
            .iter()
            .map(|code| {
                (
                    (*code).to_owned(),
                    DataType {
                        code: (*code).to_owned(),
                        url: String::new(),
                        name: String::new(),
                        components: Vec::new(),
                    },
                )
            })
            .collect()
    }

    fn base(code: &str, table: Option<&str>) -> Base {
        Base {
            code: code.to_owned(),
            table: table.map(str::to_owned),
        }
    }

    #[test]
    fn a_version_specific_code_resolves_to_the_type_it_names() {
        let own: BTreeSet<&str> = ["CE", "CE_0051", "CM_MSG", "TS"].into();
        let current = current(&["MSG", "DTM", "CWE"]);
        assert_eq!(base_of("CM_MSG", &own, &current), Some(base("MSG", None)));
        assert_eq!(
            base_of("CE_0051", &own, &current),
            Some(base("CE", Some("0051")))
        );
        assert_eq!(base_of("TS", &own, &current), Some(base("DTM", None)));
    }

    #[test]
    fn a_code_that_names_no_defined_type_has_no_base() {
        let own: BTreeSet<&str> = ["CE", "CM_ABS_RANGE", "CE_0136+0262+0263", "CK_PAT_ID"].into();
        let current = current(&["MSG", "DTM"]);
        assert_eq!(base_of("CM_ABS_RANGE", &own, &current), None);
        assert_eq!(base_of("CE_0136+0262+0263", &own, &current), None);
        assert_eq!(base_of("CK_PAT_ID", &own, &current), None);
        assert_eq!(base_of("CE", &own, &current), None);
        assert_eq!(base_of("XX_0051", &own, &current), None);
    }
}
