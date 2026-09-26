// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What this build is: the commit, the build instant, the compiler, and the
//! pins the process serves.
//!
//! The build script captures the first three and the locked versions of the
//! `openehr-*` crates and `fhir-types`; every other pin is read from the
//! constant of the crate that implements it, so the banner, the boot event and
//! `GET /health/info` never re-type a version. No specification governs this:
//! our own design.

use serde::Serialize;

/// The full git commit this binary was built from, or `unknown` outside a
/// checkout.
pub const COMMIT: &str = env!("FERROBRIDGE_BUILD_COMMIT");

/// The git commit abbreviated to twelve characters, or `unknown`.
pub const COMMIT_SHORT: &str = env!("FERROBRIDGE_BUILD_COMMIT_SHORT");

/// The build instant as an RFC 3339 UTC timestamp.
///
/// `SOURCE_DATE_EPOCH` fixes it when set, so a reproducible build reports the
/// instant of its source rather than of its compilation.
pub const BUILT_AT: &str = env!("FERROBRIDGE_BUILD_TIMESTAMP");

/// What `rustc --version` answered for the compiler that built this binary.
pub const RUSTC: &str = env!("FERROBRIDGE_BUILD_RUSTC");

/// The version `Cargo.lock` locks for the published `openehr-*` crates.
///
/// One version on the one line they are published on; two or more lines are
/// joined with `/`, so a split is visible rather than hidden.
pub const OPENEHR_CRATES: &str = env!("FERROBRIDGE_BUILD_OPENEHR_CRATES");

/// The version `Cargo.lock` locks for `fhir-types`.
pub const FHIR_TYPES: &str = env!("FERROBRIDGE_BUILD_FHIR_TYPES");

/// One pin the process serves: a name and the version it is at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pin {
    /// What is pinned, as the pin matrix names it.
    pub name: &'static str,
    /// The version, as the implementing crate states it.
    pub version: String,
}

/// Returns the pins the process serves, in the order the banner prints them.
///
/// The two mapping grammars are stated as `<language>/<version>` by their
/// crates; the version after the slash is the pin.
#[must_use]
pub fn pins() -> Vec<Pin> {
    let pin = |name, version: &str| Pin {
        name,
        version: version.to_owned(),
    };
    vec![
        pin("FHIRconnect", grammar_version(fhirconnect::GRAMMAR)),
        pin(
            "FHIR",
            &format!("R4 ({})", crate::facade::capability::FHIR_VERSION),
        ),
        pin("OMOCL", grammar_version(omocl::GRAMMAR)),
        pin("OMOP CDM", &format!("v{}", omop_cdm::CDM_VERSION)),
        pin("openEHR ITS-REST", crate::cdr::ITS_REST_VERSION),
        pin("openehr-* crates", OPENEHR_CRATES),
        pin("fhir-types", FHIR_TYPES),
    ]
}

/// Returns the version part of a `<language>/<version>` grammar identifier.
fn grammar_version(grammar: &str) -> &str {
    grammar
        .rsplit_once('/')
        .map_or(grammar, |(_, version)| version)
}

/// What `GET /health/info` answers with.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    /// The product name.
    pub product: &'static str,
    /// The product version.
    pub version: &'static str,
    /// The build facts.
    pub build: Build,
    /// The pins the process serves.
    pub pins: Vec<Pin>,
    /// The lanes this deployment configures, in banner order.
    pub lanes: Vec<LaneInfo>,
}

/// One lane, as `GET /health/info` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaneInfo {
    /// The lane's name, as the banner names it.
    pub name: &'static str,
    /// Whether the configuration switches it on.
    pub enabled: bool,
    /// The socket address a face with a listener of its own binds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    /// Whether that listener accepts connections now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up: Option<bool>,
}

/// The build facts, as `GET /health/info` reports them.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Build {
    /// The full git commit, or `unknown`.
    pub commit: &'static str,
    /// The abbreviated git commit, or `unknown`.
    pub commit_short: &'static str,
    /// The build instant, RFC 3339 UTC.
    pub built_at: &'static str,
    /// The compiler.
    pub rustc: &'static str,
}

impl Info {
    /// Returns the facts of this build.
    #[must_use]
    pub fn current() -> Self {
        Self {
            product: crate::state::PRODUCT,
            version: crate::state::VERSION,
            build: Build {
                commit: COMMIT,
                commit_short: COMMIT_SHORT,
                built_at: BUILT_AT,
                rustc: RUSTC,
            },
            pins: pins(),
            lanes: Vec::new(),
        }
    }

    /// Returns these facts with `lanes` as the lanes reported.
    #[must_use]
    pub fn with_lanes(mut self, lanes: Vec<LaneInfo>) -> Self {
        self.lanes = lanes;
        self
    }
}

/// Logs the build facts as one structured event.
pub fn log() {
    tracing::info!(
        version = crate::state::VERSION,
        commit = COMMIT_SHORT,
        built_at = BUILT_AT,
        rustc = RUSTC,
        openehr_crates = OPENEHR_CRATES,
        fhir_types = FHIR_TYPES,
        "build"
    );
}

#[cfg(test)]
mod tests {
    use super::{BUILT_AT, COMMIT, COMMIT_SHORT, Info, grammar_version, pins};

    #[test]
    fn the_grammar_version_is_the_part_after_the_slash() {
        assert_eq!("v1.0.0", grammar_version("FHIRConnect/v1.0.0"));
        assert_eq!("v2", grammar_version("v2"));
    }

    #[test]
    fn the_commit_is_hex_or_unknown_and_the_short_form_prefixes_the_full_one() {
        if COMMIT == "unknown" {
            assert_eq!("unknown", COMMIT_SHORT);
        } else {
            assert!(COMMIT.chars().all(|c| c.is_ascii_hexdigit()), "{COMMIT}");
            assert!(COMMIT.starts_with(COMMIT_SHORT), "{COMMIT_SHORT}");
        }
    }

    #[test]
    fn the_build_instant_is_an_rfc_3339_utc_timestamp() {
        let parsed: Result<jiff::Timestamp, _> = BUILT_AT.parse();
        assert!(parsed.is_ok(), "{BUILT_AT}");
        assert!(BUILT_AT.ends_with('Z'), "{BUILT_AT}");
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn every_pin_equals_its_row_in_the_pin_matrix() -> Result<(), ferrobridge_testkit::PinError> {
        let rows = [
            ("FHIRconnect", "FHIRconnect"),
            ("FHIR", "FHIR"),
            ("OMOCL", "OMOCL"),
            ("OMOP CDM", "OMOP CDM"),
            ("openEHR ITS-REST", "openEHR ITS-REST"),
            ("openehr-* crates", "openehr-rm"),
            ("fhir-types", "fhir-types"),
        ];
        let served = pins();
        assert_eq!(rows.len(), served.len(), "every pin has a matrix row");
        for (pin, (name, row)) in served.iter().zip(rows) {
            assert_eq!(name, pin.name);
            let matrix = ferrobridge_testkit::matrix_pin(row)?;
            let first = pin.version.split_whitespace().next().unwrap_or_default();
            assert_eq!(
                matrix, first,
                "the {name} pin and its docs/VERSIONS.md row disagree"
            );
        }
        let r4 = ferrobridge_testkit::matrix_pin("hl7.fhir.r4.core")?;
        assert_eq!(r4, crate::facade::capability::FHIR_VERSION);
        Ok(())
    }

    #[test]
    fn the_info_document_carries_the_build_and_every_pin() {
        let document = serde_json::to_value(Info::current()).expect("the info serializes");
        assert_eq!(Some("FerroBRIDGE"), document["product"].as_str());
        assert_eq!(Some(COMMIT), document["build"]["commit"].as_str());
        assert_eq!(
            Some(pins().len()),
            document["pins"].as_array().map(Vec::len)
        );
    }
}
