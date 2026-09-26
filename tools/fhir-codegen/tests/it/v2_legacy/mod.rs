// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The legacy half of the `v2` root set: `hl7v2_types::legacy` against the
//! fetched IGAMT tables and HL7 table 0354.
//!
//! The agreement tests read each table as plain JSON, apart from the
//! generator's own projection, and compare it with the lowered model and the
//! emitted statics.
#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::missing_assert_message,
    reason = "test assertions: the helpers index plain JSON and fail the test that calls them"
)]

mod fields;
mod links;
mod refusals;
mod root_set;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

use fhir_codegen::v2::legacy::lower::LegacyModel;
use fhir_codegen::v2::legacy::source::{TABLES_DIR, Table0354, Tables};
use serde_json::Value;

use crate::v2::MODEL;
use crate::vendor_dir;

fn legacy_dir() -> PathBuf {
    vendor_dir().join("hl7-v2-legacy")
}

fn tables_dir() -> PathBuf {
    legacy_dir().join(TABLES_DIR)
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    Tables::open(legacy_dir())
        .expect("the legacy tables should be fetched (scripts/vendor/v2-legacy.sh) and load")
});

static TABLE: LazyLock<Table0354> =
    LazyLock::new(|| Table0354::open(vendor_dir()).expect("table 0354 loads"));

static LEGACY: LazyLock<LegacyModel> =
    LazyLock::new(|| LegacyModel::lower(&TABLES, &MODEL, &TABLE).expect("the legacy tables lower"));

fn raw(version: &str, table: &str) -> Vec<Value> {
    let text = fs::read_to_string(tables_dir().join(version).join(format!("{table}.json")))
        .expect("table readable");
    serde_json::from_str::<Vec<Value>>(&text).expect("a JSON array")
}

/// The structure codes of the v2.9.1 definitions, variants aside.
fn current_codes() -> BTreeSet<String> {
    MODEL
        .structures
        .keys()
        .map(|id| match id.rsplit_once('-') {
            Some((base, variant)) if variant.len() == 1 => base.to_owned(),
            _ => id.clone(),
        })
        .collect()
}

/// Every structure a version's `messages.json` lists and its `groups.json`
/// gives a root, by id, with the versions.
fn rooted_structures() -> BTreeMap<String, BTreeSet<String>> {
    let mut rooted_at: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for version in TABLES.versions() {
        let groups = raw(&version.version, "groups");
        for message in raw(&version.version, "messages") {
            let id = message["id"].as_str().expect("id");
            let rooted = groups
                .iter()
                .any(|group| group["message_id"] == id && group["is_root"] == true);
            if rooted {
                rooted_at
                    .entry(id.to_owned())
                    .or_default()
                    .insert(version.version.clone());
            }
        }
    }
    rooted_at
}

/// The lowered structures by id, with the versions.
fn lowered_structures() -> BTreeMap<String, BTreeSet<String>> {
    let mut lowered: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for version in &LEGACY.versions {
        for id in version.structures.keys() {
            lowered
                .entry(id.clone())
                .or_default()
                .insert(version.version.clone());
        }
    }
    lowered
}
