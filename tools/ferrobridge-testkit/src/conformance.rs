// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The conformance instrument's test side: one verdict per corpus case, the
//! machine-readable result the gate reads, and the committed pass list the
//! verdicts ratchet against.
//!
//! A corpus test builds one [`Case`] per case of its corpus and hands them to
//! [`record`]. That writes `<out>/<corpus>.json` (the out directory is
//! `FERROBRIDGE_CONFORMANCE_OUT`, or `target/conformance` at the workspace
//! root), and then either rewrites `conformance/<corpus>/pass-list.txt` when
//! `FERROBRIDGE_CONFORMANCE_UPDATE` is `1`, or compares the verdicts with it.
//! The comparison answers the cases the list records as passing that no
//! longer pass, which the test asserts empty, and the cases that pass without
//! being listed, which `scripts/checks/conformance.sh` reports.
//!
//! No specification governs the instrument: our own design.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

/// The workspace root, relative to this crate's manifest.
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The environment variable that names the directory the results go to.
pub const OUT_VARIABLE: &str = "FERROBRIDGE_CONFORMANCE_OUT";

/// The environment variable that, set to `1`, rewrites the pass lists.
pub const UPDATE_VARIABLE: &str = "FERROBRIDGE_CONFORMANCE_UPDATE";

/// The prefix of the one line of a pass list that records the case count.
const TOTAL: &str = "total ";

/// One corpus the instrument measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Corpus {
    /// The vendored FHIRconnect mapping library, one case per file.
    FhirconnectMappingLib,
    /// The vendored OMOCL mapping library, one case per file.
    Omocl,
    /// The FHIR round-trip fixtures, one case per chain.
    Roundtrip,
    /// The draft FHIRconnect REST API chapter, one case per FSH operation
    /// definition.
    DraftRestApi,
    /// The vendored HL7 v2 message sets, one case per message.
    Hl7v2,
    /// The HL7 v2 message sets fetched at build time, one case per message.
    Hl7v2Smoke,
    /// The FHIR R4 examples package through `fhir-types`, one case per file.
    FhirR4,
    /// The FHIR R4B examples package through `fhir-types`, one case per file.
    FhirR4b,
    /// The FHIR R5 examples package through `fhir-types`, one case per file.
    FhirR5,
    /// The FHIR R6 ballot examples package through `fhir-types`, one case per
    /// file.
    FhirR6,
    /// The FHIR R4 examples the facade serves, one case per context and file.
    FhirR4Facade,
}

impl Corpus {
    /// Every corpus, in the order the gate reports them.
    pub const ALL: [Self; 11] = [
        Self::FhirconnectMappingLib,
        Self::Omocl,
        Self::Roundtrip,
        Self::DraftRestApi,
        Self::Hl7v2,
        Self::Hl7v2Smoke,
        Self::FhirR4,
        Self::FhirR4b,
        Self::FhirR5,
        Self::FhirR6,
        Self::FhirR4Facade,
    ];

    /// Returns the corpus identifier, which names its directory under
    /// `conformance/` and its result file.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::FhirconnectMappingLib => "fhirconnect-mapping-lib",
            Self::Omocl => "omocl",
            Self::Roundtrip => "roundtrip",
            Self::DraftRestApi => "draft-rest-api",
            Self::Hl7v2 => "hl7v2",
            Self::Hl7v2Smoke => "hl7v2-smoke",
            Self::FhirR4 => "fhir-r4",
            Self::FhirR4b => "fhir-r4b",
            Self::FhirR5 => "fhir-r5",
            Self::FhirR6 => "fhir-r6",
            Self::FhirR4Facade => "fhir-r4-facade",
        }
    }

    /// Returns the committed pass list of the corpus.
    #[must_use]
    pub fn pass_list(self) -> PathBuf {
        Path::new(ROOT)
            .join("conformance")
            .join(self.id())
            .join("pass-list.txt")
    }
}

impl fmt::Display for Corpus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// The verdict on one case of a corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    /// The case identity: a path relative to the corpus, or a chain name.
    id: String,
    /// Why the case fails, or `None` when it passes.
    failure: Option<String>,
    /// What the case counted beside its verdict, by kind.
    outcomes: BTreeMap<String, usize>,
    /// The HL7 v2 message family of the case, or `None` outside those corpora.
    family: Option<String>,
    /// The HL7 v2 version the case declares, or `None` outside those corpora.
    version: Option<String>,
}

impl Case {
    /// Returns a passing case.
    #[must_use]
    pub fn pass(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            failure: None,
            outcomes: BTreeMap::new(),
            family: None,
            version: None,
        }
    }

    /// Returns a failing case and the first reason it fails.
    #[must_use]
    pub fn fail(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            failure: Some(reason.into()),
            outcomes: BTreeMap::new(),
            family: None,
            version: None,
        }
    }

    /// Returns the case with `outcomes` recorded beside its verdict.
    ///
    /// The counts go into the result the gate reads and never decide the
    /// verdict, so a corpus can report what a passing case did not carry.
    #[must_use]
    pub fn with_outcomes(mut self, outcomes: BTreeMap<String, usize>) -> Self {
        self.outcomes = outcomes;
        self
    }

    /// Returns the case with the HL7 v2 message family it belongs to, such as
    /// `ADT`, which the gate counts its family badges by.
    #[must_use]
    pub fn with_family(mut self, family: impl Into<String>) -> Self {
        self.family = Some(family.into());
        self
    }

    /// Returns the case with the HL7 v2 version its message declares, such as
    /// `2.5.1`, which the gate counts its version badges by.
    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Returns the HL7 v2 message family of the case, when it has one.
    #[must_use]
    pub fn family(&self) -> Option<&str> {
        self.family.as_deref()
    }

    /// Returns the HL7 v2 version of the case, when it has one.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns what the case counted beside its verdict.
    #[must_use]
    pub const fn outcomes(&self) -> &BTreeMap<String, usize> {
        &self.outcomes
    }

    /// Returns the case identity.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Answers whether the case passes.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.failure.is_none()
    }

    /// Returns why the case fails, or `None` when it passes.
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
}

/// What a corpus run measured against its committed pass list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Cases the list records as passing that fail now, or that the corpus no
    /// longer holds, each with its reason.
    pub regressed: Vec<(String, String)>,
    /// Cases that pass and the list does not record.
    pub newly_passing: Vec<String>,
}

/// The instrument could not record a corpus.
#[derive(Debug, thiserror::Error)]
pub enum ConformanceError {
    /// Two cases carry one identity.
    #[error("the {corpus} corpus reports the case `{id}` twice")]
    DuplicateCase {
        /// The corpus.
        corpus: Corpus,
        /// The repeated identity.
        id: String,
    },
    /// A case identity is empty or carries white space, which the one-id-per-
    /// line list cannot hold.
    #[error("the {corpus} corpus reports the malformed case id {id:?}")]
    MalformedId {
        /// The corpus.
        corpus: Corpus,
        /// The identity as reported.
        id: String,
    },
    /// A file could not be read or written.
    #[error("cannot access {}", path.display())]
    Io {
        /// The path that was tried.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The result could not be rendered as JSON.
    #[error("cannot render the {corpus} result")]
    Json {
        /// The corpus.
        corpus: Corpus,
        /// The underlying serializer error.
        #[source]
        source: serde_json::Error,
    },
    /// The total line of a pass list carries no count.
    #[error("{} line {line} is a total line without a count", path.display())]
    MalformedTotal {
        /// The list.
        path: PathBuf,
        /// The one-based line number.
        line: usize,
        /// The underlying parse error.
        #[source]
        source: std::num::ParseIntError,
    },
    /// The committed pass list is not one id per line and one total line.
    #[error("{} line {line} is not a case id or the total line", path.display())]
    MalformedList {
        /// The list.
        path: PathBuf,
        /// The one-based line number.
        line: usize,
    },
}

/// Records the verdicts of one corpus and compares them with its pass list.
///
/// # Errors
///
/// Returns [`ConformanceError::DuplicateCase`] or
/// [`ConformanceError::MalformedId`] when the cases cannot form a list,
/// [`ConformanceError::Io`] when the result or the list cannot be written or
/// read, [`ConformanceError::Json`] when the result cannot be rendered, and
/// [`ConformanceError::MalformedList`] when the committed list is malformed.
pub fn record(corpus: Corpus, cases: &[Case]) -> Result<Outcome, ConformanceError> {
    record_set_aside(corpus, cases, &BTreeMap::new())
}

/// Records the verdicts of one corpus as [`record`] does, with the inputs the
/// corpus read and made no case of, counted by kind.
///
/// A set-aside input is neither a pass nor a failure: the corpus holds it and
/// the measured surface does not reach it, such as an example of a resource
/// type no loaded mapping serves. The counts go into the result beside the
/// cases and never into the pass list.
///
/// # Errors
///
/// Returns what [`record`] returns.
pub fn record_set_aside(
    corpus: Corpus,
    cases: &[Case],
    set_aside: &BTreeMap<String, usize>,
) -> Result<Outcome, ConformanceError> {
    let mut verdicts: BTreeMap<&str, &Case> = BTreeMap::new();
    for case in cases {
        if case.id.is_empty() || case.id.chars().any(char::is_whitespace) {
            return Err(ConformanceError::MalformedId {
                corpus,
                id: case.id.clone(),
            });
        }
        if verdicts.insert(case.id.as_str(), case).is_some() {
            return Err(ConformanceError::DuplicateCase {
                corpus,
                id: case.id.clone(),
            });
        }
    }
    write_result(corpus, &verdicts, set_aside)?;
    let list = corpus.pass_list();
    if std::env::var(UPDATE_VARIABLE).is_ok_and(|value| value == "1") {
        write_list(&list, &verdicts)?;
    }
    let listed = read_list(&list)?;
    Ok(compare(&verdicts, &listed))
}

/// Compares the verdicts with the ids a list records as passing.
fn compare(verdicts: &BTreeMap<&str, &Case>, listed: &BTreeSet<String>) -> Outcome {
    let regressed = listed
        .iter()
        .filter_map(|id| match verdicts.get(id.as_str()) {
            Some(case) => case.failure().map(|reason| (id.clone(), reason.to_owned())),
            None => Some((id.clone(), String::from("the corpus no longer holds it"))),
        })
        .collect();
    let newly_passing = verdicts
        .iter()
        .filter(|&(id, case)| case.passed() && !listed.contains(*id))
        .map(|(id, _)| (*id).to_owned())
        .collect();
    Outcome {
        regressed,
        newly_passing,
    }
}

/// Returns the directory the results go to.
fn out_directory() -> PathBuf {
    std::env::var_os(OUT_VARIABLE).map_or_else(
        || Path::new(ROOT).join("target").join("conformance"),
        PathBuf::from,
    )
}

/// Writes `<out>/<corpus>.json`.
fn write_result(
    corpus: Corpus,
    verdicts: &BTreeMap<&str, &Case>,
    set_aside: &BTreeMap<String, usize>,
) -> Result<(), ConformanceError> {
    let rendered: Vec<serde_json::Value> = verdicts
        .values()
        .map(|case| {
            serde_json::json!({
                "id": case.id,
                "passed": case.passed(),
                "failure": case.failure,
                "outcomes": case.outcomes,
                "family": case.family,
                "version": case.version,
            })
        })
        .collect();
    let document = serde_json::json!({
        "corpus": corpus.id(),
        "total": verdicts.len(),
        "set_aside": set_aside,
        "cases": rendered,
    });
    let mut text = serde_json::to_string_pretty(&document)
        .map_err(|source| ConformanceError::Json { corpus, source })?;
    text.push('\n');
    let directory = out_directory();
    std::fs::create_dir_all(&directory).map_err(|source| ConformanceError::Io {
        path: directory.clone(),
        source,
    })?;
    let path = directory.join(format!("{}.json", corpus.id()));
    std::fs::write(&path, text).map_err(|source| ConformanceError::Io { path, source })
}

/// Rewrites a pass list: every passing id in byte order, then the total line.
fn write_list(path: &Path, verdicts: &BTreeMap<&str, &Case>) -> Result<(), ConformanceError> {
    let mut text = String::new();
    for case in verdicts.values().filter(|case| case.passed()) {
        text.push_str(&case.id);
        text.push('\n');
    }
    text.push_str(TOTAL);
    text.push_str(&verdicts.len().to_string());
    text.push('\n');
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(|source| ConformanceError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, text).map_err(|source| ConformanceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Reads the ids a pass list records, leaving its total line aside.
fn read_list(path: &Path) -> Result<BTreeSet<String>, ConformanceError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConformanceError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_list(path, &text)
}

/// Parses the text of a pass list.
fn parse_list(path: &Path, text: &str) -> Result<BTreeSet<String>, ConformanceError> {
    let mut listed = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let malformed = || ConformanceError::MalformedList {
            path: path.to_path_buf(),
            line: index.saturating_add(1),
        };
        if let Some(count) = line.strip_prefix(TOTAL) {
            count
                .parse::<usize>()
                .map_err(|source| ConformanceError::MalformedTotal {
                    path: path.to_path_buf(),
                    line: index.saturating_add(1),
                    source,
                })?;
            continue;
        }
        if line.is_empty() || line.chars().any(char::is_whitespace) {
            return Err(malformed());
        }
        listed.insert(line.to_owned());
    }
    Ok(listed)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::{Case, ConformanceError, Corpus, compare, parse_list};

    fn verdicts(cases: &[Case]) -> BTreeMap<&str, &Case> {
        cases.iter().map(|case| (case.id(), case)).collect()
    }

    #[test]
    fn a_listed_case_that_fails_now_is_a_regression_with_its_reason() {
        let cases = [Case::pass("a.yml"), Case::fail("b.yml", "refused")];
        let listed: BTreeSet<String> = ["a.yml", "b.yml"].map(String::from).into();
        let outcome = compare(&verdicts(&cases), &listed);
        assert_eq!(
            outcome.regressed,
            vec![(String::from("b.yml"), String::from("refused"))]
        );
        assert!(outcome.newly_passing.is_empty());
    }

    #[test]
    fn a_listed_case_the_corpus_lost_is_a_regression() {
        let cases = [Case::pass("a.yml")];
        let listed: BTreeSet<String> = ["a.yml", "gone.yml"].map(String::from).into();
        let outcome = compare(&verdicts(&cases), &listed);
        assert_eq!(outcome.regressed.len(), 1);
        assert_eq!(
            outcome.regressed.first().map(|(id, _)| id.as_str()),
            Some("gone.yml")
        );
    }

    #[test]
    fn an_unlisted_passing_case_is_newly_passing_and_no_regression() {
        let cases = [Case::pass("a.yml"), Case::pass("c.yml")];
        let listed: BTreeSet<String> = ["a.yml"].map(String::from).into();
        let outcome = compare(&verdicts(&cases), &listed);
        assert!(outcome.regressed.is_empty());
        assert_eq!(outcome.newly_passing, vec![String::from("c.yml")]);
    }

    #[test]
    fn a_list_reads_its_ids_and_leaves_the_total_aside() {
        let listed = parse_list(Path::new("pass-list.txt"), "a.yml\nb/c.yml\ntotal 3\n");
        assert_eq!(
            listed.ok(),
            Some(["a.yml", "b/c.yml"].map(String::from).into())
        );
    }

    #[test]
    fn a_line_with_white_space_is_refused_by_its_line_number() {
        let refused = parse_list(Path::new("pass-list.txt"), "a.yml\nb c\n");
        assert!(matches!(
            refused,
            Err(ConformanceError::MalformedList { line: 2, .. })
        ));
    }

    #[test]
    fn a_total_line_without_a_count_is_refused() {
        let refused = parse_list(Path::new("pass-list.txt"), "total many\n");
        assert!(matches!(
            refused,
            Err(ConformanceError::MalformedTotal { line: 1, .. })
        ));
    }

    #[test]
    fn outcomes_travel_with_a_case_and_leave_its_verdict_alone() {
        let outcomes: BTreeMap<String, usize> = [(String::from("unmapped-field"), 3)].into();
        let case = Case::pass("a.hl7").with_outcomes(outcomes.clone());
        assert!(case.passed());
        assert_eq!(case.outcomes(), &outcomes);
        let failed = Case::fail("b.hl7", "refused").with_outcomes(outcomes);
        assert_eq!(failed.failure(), Some("refused"));
    }

    #[test]
    fn a_case_carries_no_family_or_version_until_it_is_given_one() {
        let plain = Case::pass("a.yml");
        assert_eq!((plain.family(), plain.version()), (None, None));
        let message = Case::fail("b.hl7", "refused")
            .with_family("ADT")
            .with_version("2.5.1");
        assert_eq!(message.family(), Some("ADT"));
        assert_eq!(message.version(), Some("2.5.1"));
        assert_eq!(message.failure(), Some("refused"));
    }

    #[test]
    fn every_corpus_names_its_own_list() {
        for corpus in Corpus::ALL {
            assert!(
                corpus
                    .pass_list()
                    .ends_with(format!("conformance/{}/pass-list.txt", corpus.id())),
                "{corpus}"
            );
        }
    }
}
