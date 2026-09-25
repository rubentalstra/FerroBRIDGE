// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Captures the build facts the banner, the boot event and `GET /health/info`
//! report: the git commit, the build instant, the `rustc` version, and the
//! versions of the `openehr-*` crates and of `fhir-types` this build locked.
//!
//! A build outside a git checkout reports the commit as `unknown`; a missing
//! or unreadable `Cargo.lock` fails the build, because the pins it carries are
//! read from nowhere else. No specification governs this: our own design.

use std::collections::BTreeSet;
use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

/// The value a fact takes when this build cannot know it.
const UNKNOWN: &str = "unknown";

fn main() -> Result<(), Box<dyn Error>> {
    let full = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| UNKNOWN.to_owned());
    let short = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| UNKNOWN.to_owned());
    println!("cargo:rustc-env=FERROBRIDGE_BUILD_COMMIT={full}");
    println!("cargo:rustc-env=FERROBRIDGE_BUILD_COMMIT_SHORT={short}");
    for path in ["HEAD", "refs", "packed-refs"] {
        if let Some(watched) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={watched}");
        }
    }

    // NOTE: the `SOURCE_DATE_EPOCH` specification
    // (<https://reproducible-builds.org/specs/source-date-epoch/>): when set,
    // it is the build instant, so two builds of one tree report the same one.
    let epoch = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => value.trim().parse::<i64>().map_err(|error| {
            format!("SOURCE_DATE_EPOCH {value:?} is not a count of seconds: {error}")
        })?,
        Err(std::env::VarError::NotPresent) => jiff::Timestamp::now().as_second(),
        Err(error) => return Err(Box::new(error)),
    };
    let built_at = jiff::Timestamp::from_second(epoch)?;
    println!("cargo:rustc-env=FERROBRIDGE_BUILD_TIMESTAMP={built_at}");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=src");

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let rustc_version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || UNKNOWN.to_owned(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        );
    println!("cargo:rustc-env=FERROBRIDGE_BUILD_RUSTC={rustc_version}");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let lock_path = manifest.join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock: toml::Table = toml::from_str(&std::fs::read_to_string(&lock_path)?)?;
    let openehr = locked_versions(&lock, |name, registry| {
        registry && name.starts_with("openehr-")
    });
    let fhir_types = locked_versions(&lock, |name, _| name == "fhir-types");
    for (key, versions) in [
        ("FERROBRIDGE_BUILD_OPENEHR_CRATES", openehr),
        ("FERROBRIDGE_BUILD_FHIR_TYPES", fhir_types),
    ] {
        if versions.is_empty() {
            return Err(format!("Cargo.lock locks no package for {key}").into());
        }
        let joined = versions.into_iter().collect::<Vec<String>>().join("/");
        println!("cargo:rustc-env={key}={joined}");
    }
    Ok(())
}

/// Runs `git` with `args` and returns its trimmed output, when it succeeds.
fn git(args: &[&str]) -> Option<String> {
    // NOTE: no specification governs this: our own design. A build outside a
    // checkout is legitimate, so a failed `git` is an absent fact.
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Returns the distinct versions `lock` holds for the packages `wanted`
/// accepts, given each package's name and whether it comes from a registry.
fn locked_versions(lock: &toml::Table, wanted: impl Fn(&str, bool) -> bool) -> BTreeSet<String> {
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    packages
        .iter()
        .filter_map(toml::Value::as_table)
        .filter_map(|package| {
            let name = package.get("name")?.as_str()?;
            let registry = package
                .get("source")
                .and_then(toml::Value::as_str)
                .is_some_and(|source| source.starts_with("registry+"));
            let version = package.get("version")?.as_str()?;
            wanted(name, registry).then(|| version.to_owned())
        })
        .collect()
}
