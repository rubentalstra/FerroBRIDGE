// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `v2` root set: `hl7v2-types` against the fetched HL7 v2 definitions.
//!
//! The agreement tests read each definition file as plain JSON, apart from
//! the generator's own projection, and compare it with the emitted statics
//! element by element.
#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::missing_assert_message,
    reason = "test assertions: the helpers index plain JSON and fail the test that calls them"
)]

mod data_types;
mod emit;
mod messages;
mod refusals;
mod segments;
mod structures;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use fhir_codegen::roots::V2RootSet;
use fhir_codegen::v2::corpus::{Corpus, SOURCE_OF_TRUTH};
use fhir_codegen::v2::lower::Model;
use hl7v2_types::model::{Cardinality, ConditionalCode, Max, Optionality};
use serde_json::Value;

use crate::vendor_dir;

fn definitions_dir() -> PathBuf {
    vendor_dir().join("hl7-v2ig")
}

fn tree_dir() -> PathBuf {
    definitions_dir().join(SOURCE_OF_TRUTH)
}

fn hl7v2_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/hl7v2-types")
}

/// The fetched definitions, loaded once.
static CORPUS: LazyLock<Corpus> = LazyLock::new(|| {
    Corpus::open(definitions_dir())
        .expect("the HL7 v2 definitions should be fetched (scripts/vendor/v2ig.sh) and load")
});

/// The lowered model, once.
pub(crate) static MODEL: LazyLock<Model> = LazyLock::new(|| {
    let roots = V2RootSet::select(&CORPUS).expect("every structure file is a message structure");
    Model::lower(&CORPUS, &roots).expect("the definitions lower")
});

fn raw(file: &str) -> Value {
    let text = fs::read_to_string(tree_dir().join(file)).expect("definition readable");
    serde_json::from_str(&text).expect("definition is JSON")
}

fn elements(value: &Value) -> Vec<Value> {
    value["differential"]["element"]
        .as_array()
        .expect("a differential")
        .clone()
}

fn json_files(dir: &str) -> Vec<String> {
    let mut files: Vec<String> = fs::read_dir(tree_dir().join(dir))
        .expect("dir listable")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "json"))
        .map(|name| format!("{dir}/{name}"))
        .collect();
    files.sort();
    files
}

fn raw_integer(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn raw_cardinality(element: &Value) -> Cardinality {
    let min = u32::try_from(element["min"].as_u64().expect("min")).expect("min fits");
    let max = match &element["max"] {
        Value::String(star) if star == "*" => Max::Unbounded,
        other => Max::Bounded(u32::try_from(raw_integer(other).expect("max")).expect("max fits")),
    };
    Cardinality { min, max }
}

fn extension<'a>(element: &'a Value, url: &str) -> Option<&'a Value> {
    element["extension"]
        .as_array()
        .and_then(|extensions| extensions.iter().find(|extension| extension["url"] == url))
}

fn nested<'a>(extension: &'a Value, url: &str) -> Option<&'a Value> {
    extension["extension"]
        .as_array()
        .and_then(|nested| nested.iter().find(|entry| entry["url"] == url))
        .map(|entry| &entry["valueInteger"])
}

fn conditional_code(code: ConditionalCode) -> &'static str {
    match code {
        ConditionalCode::R => "R",
        ConditionalCode::Re => "RE",
        ConditionalCode::O => "O",
        ConditionalCode::X => "X",
    }
}

fn optionality_code(optionality: Optionality) -> String {
    match optionality {
        Optionality::R => String::from("R"),
        Optionality::Re => String::from("RE"),
        Optionality::O => String::from("O"),
        Optionality::C => String::from("C"),
        Optionality::Conditional { first, second } => {
            format!(
                "C({}/{})",
                conditional_code(first),
                conditional_code(second)
            )
        }
        Optionality::X => String::from("X"),
        Optionality::B => String::from("B"),
        Optionality::W => String::from("W"),
        Optionality::Na => String::from("NA"),
        Optionality::Unstated => String::from("-"),
    }
}
