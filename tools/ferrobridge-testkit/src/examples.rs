// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HL7 examples package of each FHIR version, the corpus of the model and
//! facade conformance tests.
//!
//! `scripts/vendor/fhir-packages.sh --build-time` fetches each package into
//! `vendor/<package>/package/` of this crate at the version `docs/VERSIONS.md`
//! pins, beside a committed `PROVENANCE.md`; the tree itself is never
//! committed. A suite asks [`Package::fetched`] first and skips when the tree
//! is absent, as the other build-time corpora do. No specification governs
//! this layout: our own design.

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

/// The directory the packages are fetched into, relative to this crate.
const VENDOR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor");

/// The two files of a package that are no example resource: the npm
/// manifest and the package index (<https://hl7.org/fhir/packages.html>).
const MANIFESTS: [&str; 2] = ["package.json", ".index.json"];

/// One FHIR version's examples package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Package {
    /// `hl7.fhir.r4.examples`, the examples of FHIR 4.0.1.
    R4,
    /// `hl7.fhir.r4b.examples`, the examples of FHIR 4.3.0.
    R4b,
    /// `hl7.fhir.r5.examples`, the examples of FHIR 5.0.0.
    R5,
    /// `hl7.fhir.r6.examples`, the examples of the R6 ballot the core package
    /// pin names.
    R6,
}

impl Package {
    /// Returns the package name, which names its directory.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::R4 => "hl7.fhir.r4.examples",
            Self::R4b => "hl7.fhir.r4b.examples",
            Self::R5 => "hl7.fhir.r5.examples",
            Self::R6 => "hl7.fhir.r6.examples",
        }
    }

    /// Returns the `package/` directory the fetch extracts the tarball into.
    #[must_use]
    pub fn directory(self) -> PathBuf {
        Path::new(VENDOR).join(self.name()).join("package")
    }

    /// Answers whether the package is on disk, with its manifest.
    #[must_use]
    pub fn fetched(self) -> bool {
        self.directory().join("package.json").is_file()
    }

    /// Returns every example resource file of the package in byte order of
    /// its name: each JSON file of `package/` but the manifests.
    ///
    /// # Errors
    ///
    /// Returns the I/O error of a directory that cannot be listed.
    pub fn resources(self) -> std::io::Result<Vec<PathBuf>> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(self.directory())? {
            let path = entry?.path();
            let name = path.file_name().and_then(std::ffi::OsStr::to_str);
            let is_json = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
            let is_resource = is_json && name.is_some_and(|name| !MANIFESTS.contains(&name));
            if is_resource && path.is_file() {
                found.push(path);
            }
        }
        found.sort();
        Ok(found)
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::Package;

    #[test]
    fn each_package_lives_under_its_own_name() {
        for package in [Package::R4, Package::R4b, Package::R5, Package::R6] {
            assert!(
                package
                    .directory()
                    .ends_with(format!("vendor/{}/package", package.name())),
                "{package}"
            );
        }
    }
}
