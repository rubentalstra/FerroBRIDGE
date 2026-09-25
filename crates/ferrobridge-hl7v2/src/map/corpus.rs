// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The v2-to-FHIR `ConceptMaps`, loaded once into rows the interpreter runs.
//!
//! The package `hl7.fhir.uv.v2mappings` ships four kinds of `ConceptMap`, named
//! by the first word of their id: `message-` (a message structure to a
//! Bundle), `segment-` (a segment to one resource), `datatype-` (a data type
//! to a FHIR type) and `table-` (a v2 table to a code system, which a
//! terminology server answers through `$translate`). Each map is decoded
//! through the generated `fhir_types::r4` `ConceptMap`, and each row's
//! condition, target notation and `TypeInfo` extension
//! (`StructureDefinition-TypeInfo.json`: `type`, `assignment`, `mappedVia`)
//! are read once here, so a form the interpreter cannot run is known before
//! a message arrives and is counted when a row reaches it.
//!
//! A supplement is a second directory of `ConceptMaps` in the same shape, loaded
//! after the package: a supplement map with the canonical url of a package map
//! replaces it whole, and any other is added. No specification governs this:
//! our own design.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fhir_types::codec::{Json, Value};
use fhir_types::r4::concept_map::{ConceptMap, ConceptMapGroupElementTarget};
use fhir_types::r4::extension::{Extension, ExtensionValue};

use crate::map::condition::{self, AssignmentError, ConditionError, Expr, Part};
use crate::map::notation::{self, NotationError, Target};

/// The canonical url of the `TypeInfo` extension.
pub const TYPE_INFO: &str = "http://hl7.org/fhir/uv/v2mappings/StructureDefinition/TypeInfo";

/// The canonical base the relative `mappedVia` urls are read against.
pub const CANONICAL_BASE: &str = "http://hl7.org/fhir/uv/v2mappings/";

/// The kind of a `ConceptMap`, from the first word of its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// `message-`.
    Message,
    /// `segment-`.
    Segment,
    /// `datatype-`.
    Datatype,
    /// `table-`.
    Table,
    /// Any other id.
    Other,
}

impl Kind {
    fn of(id: &str) -> Self {
        match id.split_once('-').map(|(head, _)| head) {
            Some("message") => Self::Message,
            Some("segment") => Self::Segment,
            Some("datatype") => Self::Datatype,
            Some("table") => Self::Table,
            _ => Self::Other,
        }
    }
}

/// What gates a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// No condition: the row always applies.
    Always,
    /// A `Computable-ANTLR` condition.
    Computable(Expr),
    /// A `Narrative-Condition`, which no machine evaluates.
    Narrative,
    /// A `Computable-ANTLR` condition whose check names no operand
    /// ([`condition::ConditionError::NoOperand`]), which no machine can
    /// evaluate.
    Defective {
        /// The condition as written.
        text: String,
    },
    /// A condition in a form the interpreter does not evaluate.
    Unsupported {
        /// The property the form came under.
        property: String,
        /// The condition as written.
        text: String,
    },
}

/// A row's `assignment`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignment {
    /// A double-quoted literal, written as given.
    Literal(String),
    /// Literals and operands joined by `+` ([`condition::assignment`]), whose
    /// texts are written joined.
    Concat(Vec<Part>),
    /// Parts with no `+` between two of them
    /// ([`condition::AssignmentError::MissingOperator`]).
    Defective(String),
    /// Any other form: an expression or a narrative instruction.
    Unsupported(String),
}

/// A row's `mappedVia`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappedVia {
    /// The id of a map, read from `ConceptMap/<id>`: a table map, or the
    /// data type map the row runs (`StructureDefinition-TypeInfo.json`,
    /// `mappedVia`: "Url of the mapping artifact for the item").
    Table(String),
    /// Any other value, such as `unspecified_mapping`.
    Unresolved(String),
}

/// One row: one source element to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The source code as written, for example `PID-5` or `CX.1`.
    pub source: String,
    /// The source's v2 data type from its `TypeInfo`.
    pub source_type: Option<String>,
    /// The target code as written.
    pub target_code: String,
    /// The parsed target.
    pub target: Result<Target, NotationError>,
    /// The target's FHIR type from its `TypeInfo`.
    pub target_type: Option<String>,
    /// The target's `assignment`.
    pub assignment: Option<Assignment>,
    /// The target's `mappedVia`.
    pub mapped_via: Option<MappedVia>,
    /// The condition gating the row.
    pub condition: Condition,
}

/// One group of a table map: the source and target code systems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableGroup {
    /// `group.source`.
    pub source: String,
    /// `group.target`.
    pub target: String,
}

/// One `ConceptMap`, read into rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map {
    /// The id.
    pub id: String,
    /// The canonical url.
    pub url: String,
    /// The kind, from the id.
    pub kind: Kind,
    /// The rows, in map order; empty for a table map.
    pub rows: Vec<Row>,
    /// The groups of a table map.
    pub groups: Vec<TableGroup>,
}

impl Map {
    /// Returns whether `other` is an alternative to this map for one value:
    /// both map one source component, with no condition, into the same child
    /// of the target, or one of them into the target itself (`$value`).
    ///
    /// The four `datatype-ei-<qualifier>-to-identifier` maps each write
    /// `EI.1` into `value` and are alternatives; maps that write distinct
    /// children, or one child only under a condition, fill parts of one
    /// target. No specification governs this: our own design, since no row
    /// of the guide names its data type map.
    #[must_use]
    pub fn alternative_to(&self, other: &Self) -> bool {
        self.rows.iter().any(|mine| {
            other.rows.iter().any(|theirs| {
                mine.source == theirs.source
                    && mine.condition == Condition::Always
                    && theirs.condition == Condition::Always
                    && match (reach(mine), reach(theirs)) {
                        (Some(Reach::Whole), Some(_)) | (Some(_), Some(Reach::Whole)) => true,
                        (Some(Reach::Child(a)), Some(Reach::Child(b))) => a == b,
                        _ => false,
                    }
            })
        })
    }
}

/// The part of its target a row writes.
enum Reach<'r> {
    /// `$value`: the target itself.
    Whole,
    /// The child with this name.
    Child(&'r str),
}

/// Returns the part of its target `row` writes, `None` for a target that
/// does not parse.
fn reach(row: &Row) -> Option<Reach<'_>> {
    match &row.target {
        Ok(Target::Value { .. }) => Some(Reach::Whole),
        Ok(Target::Path { steps, .. }) => steps.first().map(|step| Reach::Child(&step.name)),
        Err(_) => None,
    }
}

/// A refusal to load the corpus.
#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    /// A directory or file could not be read.
    #[error("{path} could not be read")]
    Read {
        /// The path.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// A file is no JSON.
    #[error("{path} is no JSON document")]
    Json {
        /// The path.
        path: PathBuf,
        /// The cause.
        #[source]
        source: serde_json::Error,
    },
    /// A file does not decode as an R4 `ConceptMap`.
    #[error("{path} does not decode as an R4 ConceptMap")]
    Decode {
        /// The path.
        path: PathBuf,
        /// The cause.
        #[source]
        source: fhir_types::codec::DecodeError,
    },
    /// A `ConceptMap` has no id or no url.
    #[error("{path} names no id or no url")]
    Unnamed {
        /// The path.
        path: PathBuf,
    },
}

/// The loaded `ConceptMaps`, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Corpus {
    maps: BTreeMap<String, Map>,
}

impl Corpus {
    /// Loads every `ConceptMap-*.json` in `directory`, in file name order.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError`] when the directory or a file cannot be read, or
    /// a file is no R4 `ConceptMap` with an id and a url.
    pub fn load(directory: &Path) -> Result<Self, CorpusError> {
        let mut corpus = Self::default();
        corpus.add_directory(directory)?;
        Ok(corpus)
    }

    /// Loads the `ConceptMaps` of a supplement directory over this corpus: a map
    /// with the url of a loaded one replaces it, any other is added.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError`] as [`Corpus::load`] does.
    pub fn supplement(mut self, directory: &Path) -> Result<Self, CorpusError> {
        self.add_directory(directory)?;
        Ok(self)
    }

    /// Returns the map with `id`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Map> {
        self.maps.get(id)
    }

    /// Returns every map, by id.
    pub fn maps(&self) -> impl Iterator<Item = &Map> {
        self.maps.values()
    }

    /// Returns the message map whose rows name segments of `structure`, for
    /// example `ORU_R01`.
    #[must_use]
    pub fn message_map(&self, structure: &str) -> Option<&Map> {
        let dotted = format!("{structure}.");
        let follow = format!("{structure}:");
        self.maps.values().find(|map| {
            map.kind == Kind::Message
                && !map.rows.is_empty()
                && map
                    .rows
                    .iter()
                    .all(|row| row.source.starts_with(&dotted) || row.source.starts_with(&follow))
        })
    }

    /// Returns the map of `kind` from `source` to `target`: the one whose id is
    /// `<kind>-<source>-to-<target>`, else the one qualified map
    /// `<kind>-<source>-<qualifier>-to-<target>` when there is exactly one.
    ///
    /// # Errors
    ///
    /// Returns the ids of the qualified candidates when there is none or
    /// several.
    pub fn find(&self, kind: &str, source: &str, target: &str) -> Result<&Map, Vec<String>> {
        match self.qualifying(kind, source, target).as_slice() {
            [one] => Ok(one),
            several => Err(several.iter().map(|map| map.id.clone()).collect()),
        }
    }

    /// Returns every map of `kind` from `source` to `target`: the one whose
    /// id is `<kind>-<source>-to-<target>` when there is one, else each
    /// qualified map `<kind>-<source>-<qualifier>-to-<target>`, by id.
    #[must_use]
    pub fn qualifying(&self, kind: &str, source: &str, target: &str) -> Vec<&Map> {
        let source = source.to_ascii_lowercase();
        let target = target.to_ascii_lowercase();
        let exact = format!("{kind}-{source}-to-{target}");
        if let Some(map) = self.maps.get(&exact) {
            return vec![map];
        }
        let prefix = format!("{kind}-{source}-");
        let suffix = format!("-to-{target}");
        self.maps
            .values()
            .filter(|map| map.id.starts_with(&prefix) && map.id.ends_with(&suffix))
            .collect()
    }

    /// Reads one directory's `ConceptMaps` into the corpus.
    fn add_directory(&mut self, directory: &Path) -> Result<(), CorpusError> {
        let read = |source| CorpusError::Read {
            path: directory.to_path_buf(),
            source,
        };
        let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
            .map_err(read)?
            .map(|entry| entry.map(|entry| entry.path()).map_err(read))
            .collect::<Result<_, _>>()?;
        paths.retain(|path| {
            let json = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
            json && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ConceptMap-"))
        });
        paths.sort();
        for path in paths {
            let map = read_map(&path)?;
            let replaced = self
                .maps
                .iter()
                .find(|(_, loaded)| loaded.url == map.url)
                .map(|(id, _)| id.clone());
            if let Some(id) = replaced {
                self.maps.remove(&id);
            }
            self.maps.insert(map.id.clone(), map);
        }
        Ok(())
    }
}

/// Reads one `ConceptMap` file.
fn read_map(path: &Path) -> Result<Map, CorpusError> {
    let bytes = std::fs::read(path).map_err(|source| CorpusError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|source| CorpusError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    let Value::Object(object) = value else {
        return Err(CorpusError::Unnamed {
            path: path.to_path_buf(),
        });
    };
    let concept_map =
        ConceptMap::from_json(&object, &mut fhir_types::codec::Path::root("ConceptMap")).map_err(
            |source| CorpusError::Decode {
                path: path.to_path_buf(),
                source,
            },
        )?;
    let unnamed = || CorpusError::Unnamed {
        path: path.to_path_buf(),
    };
    let id = concept_map.id.clone().ok_or_else(unnamed)?;
    let url = concept_map
        .url
        .as_ref()
        .and_then(|url| url.value.clone())
        .ok_or_else(unnamed)?;
    let kind = Kind::of(&id);
    let mut rows = Vec::new();
    let mut groups = Vec::new();
    for group in &concept_map.group {
        if kind == Kind::Table {
            let text = |value: Option<&fhir_types::r4::primitives::Uri>| {
                value.and_then(|uri| uri.value.clone()).unwrap_or_default()
            };
            groups.push(TableGroup {
                source: text(group.source.as_ref()),
                target: text(group.target.as_ref()),
            });
            continue;
        }
        for element in &group.element {
            let source = element
                .code
                .as_ref()
                .and_then(|code| code.value.clone())
                .unwrap_or_default();
            let source_type = type_info(&element.extension, "type");
            for target in &element.target {
                rows.push(row(&source, source_type.clone(), target));
            }
        }
    }
    Ok(Map {
        id,
        url,
        kind,
        rows,
        groups,
    })
}

/// Reads one target into a row.
fn row(source: &str, source_type: Option<String>, target: &ConceptMapGroupElementTarget) -> Row {
    let target_code = target
        .code
        .as_ref()
        .and_then(|code| code.value.clone())
        .unwrap_or_default();
    let assignment = type_info(&target.extension, "assignment").map(|text| {
        match text
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .filter(|inner| !inner.contains('"'))
        {
            Some(literal) => Assignment::Literal(String::from(literal)),
            None => match condition::assignment(&text) {
                Ok(parts) => Assignment::Concat(parts),
                Err(AssignmentError::MissingOperator { .. }) => Assignment::Defective(text),
                Err(AssignmentError::Lex { .. } | AssignmentError::Syntax { .. }) => {
                    Assignment::Unsupported(text)
                }
            },
        }
    });
    let mapped_via = type_info(&target.extension, "mappedVia").map(|text| {
        match text.strip_prefix("ConceptMap/") {
            Some(id) if !id.is_empty() => MappedVia::Table(String::from(id)),
            _ => MappedVia::Unresolved(text),
        }
    });
    Row {
        source: String::from(source),
        source_type,
        target: notation::parse(&target_code),
        target_code,
        target_type: type_info(&target.extension, "type"),
        assignment,
        mapped_via,
        condition: condition(source, target),
    }
}

/// Reads the condition of a target from its `dependsOn` entries.
///
/// A `Narrative-Condition` beside a computable one still gates the row on a
/// judgement no machine makes, so the row is narrative. A `Computable-ANTLR`
/// and a `Computable-FHIRPath` on one row state one condition twice, and the
/// ANTLR form is the one evaluated; a `Computable-FHIRPath` alone is refused,
/// since it is FHIRPath over the v2 message, not over a FHIR resource. No
/// specification governs the precedence: our own design.
fn condition(source: &str, target: &ConceptMapGroupElementTarget) -> Condition {
    let property = |name: &str| {
        target.depends_on.iter().find_map(|entry| {
            (entry.property.value.as_deref() == Some(name))
                .then(|| entry.value.value.clone().unwrap_or_default())
        })
    };
    if property("Narrative-Condition").is_some() {
        return Condition::Narrative;
    }
    if let Some(text) = property("Computable-ANTLR") {
        let own = condition::source_operand(source);
        return match condition::parse_for(&text, own.as_ref()) {
            Ok(expr) => Condition::Computable(expr),
            Err(ConditionError::NoOperand { .. }) => Condition::Defective { text },
            Err(ConditionError::Lex { .. } | ConditionError::Syntax { .. }) => {
                Condition::Unsupported {
                    property: String::from("Computable-ANTLR"),
                    text,
                }
            }
        };
    }
    if let Some(text) = property("Computable-FHIRPath") {
        return Condition::Unsupported {
            property: String::from("Computable-FHIRPath"),
            text,
        };
    }
    match target.depends_on.first() {
        Some(entry) => Condition::Unsupported {
            property: entry.property.value.clone().unwrap_or_default(),
            text: entry.value.value.clone().unwrap_or_default(),
        },
        None => Condition::Always,
    }
}

/// Reads the `TypeInfo` sub-extension `name` as text.
fn type_info(extensions: &[Extension], name: &str) -> Option<String> {
    extensions
        .iter()
        .filter(|extension| extension.url == TYPE_INFO)
        .flat_map(|extension| extension.extension.iter())
        .find(|inner| inner.url == name)
        .and_then(|inner| match &inner.value {
            Some(ExtensionValue::Code(code)) => code.value.clone(),
            Some(ExtensionValue::String(text)) => text.value.clone(),
            Some(ExtensionValue::Url(url)) => url.value.clone(),
            Some(ExtensionValue::Uri(uri)) => uri.value.clone(),
            _ => None,
        })
}
