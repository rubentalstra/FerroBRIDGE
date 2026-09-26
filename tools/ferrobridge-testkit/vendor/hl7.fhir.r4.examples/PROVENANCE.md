<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: hl7.fhir.r4.examples

Fetched verbatim at build time by `scripts/vendor/fhir-packages.sh
--build-time` into `package/` beside this file, which `.gitignore` refuses
(.claude/rules/vendored-inputs.md). Never commit a file from there and never
edit one: change the pin in docs/VERSIONS.md, run the script with
`--build-time --stamp`, and record the count and digest it prints.

- Package: hl7.fhir.r4.examples
- Version: 4.0.1
- Source: the FHIR package registry, https://packages.fhir.org/hl7.fhir.r4.examples
- Tarball: https://packages.simplifier.net/hl7.fhir.r4.examples/4.0.1
- SHA-1 (registry shasum): ef0ea43649d05ff246b471cad4583b662f1f27b5
- SHA-256 (tarball): e18b31e7a52145a31f1f3f409cf6847583b08b7306cc4e9460a95a7b5efba930
- Upstream license: CC0-1.0 (the `license` field of `package/package.json`)
- Layout: the tarball's `package/` directory, extracted unchanged
- Files: 5311
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `51f5c093f9f197fcab683ac1bc3380e44b1c7e37c6c74baeefff7fa3eb8b7c1e`
- Contents: 5309 resources of 140 resource types, one JSON file each,
  beside `package.json` and `.index.json`
- Stamped: 2026-09-26

## Why build time

The four examples packages unpack to about 600 MB, which would more than
double the 380 MB of FHIR packages that `tools/fhir-codegen/vendor/` commits,
for corpora only the conformance tests read. The licence would permit
vendoring; the size decides. CI restores the trees from a cache keyed on
`scripts/vendor/fhir-packages.sh --cache-key`, and the script verifies the
count and digest of a restored tree as it does of a download.

The examples are HL7's own published examples, never patient data.
