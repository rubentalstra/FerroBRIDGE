<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: hl7.fhir.r4b.examples

Fetched verbatim at build time by `scripts/vendor/fhir-packages.sh
--build-time` into `package/` beside this file, which `.gitignore` refuses
(.claude/rules/vendored-inputs.md). Never commit a file from there and never
edit one: change the pin in docs/VERSIONS.md, run the script with
`--build-time --stamp`, and record the count and digest it prints.

- Package: hl7.fhir.r4b.examples
- Version: 4.3.0
- Source: the FHIR package registry, https://packages.fhir.org/hl7.fhir.r4b.examples
- Tarball: https://packages.simplifier.net/hl7.fhir.r4b.examples/4.3.0
- SHA-1 (registry shasum): ee58a0ee13a82944d9b1902a33b6376e9c7aa7e6
- SHA-256 (tarball): a64db561a943eab454d85349cfb5a088cbfc57c3d90706db36ae3df64269afa5
- Upstream license: CC0-1.0 (the `license` field of `package/package.json`)
- Layout: the tarball's `package/` directory, extracted unchanged
- Files: 2842
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `7c42275e73bff53eef52c9f48e3bd82d3eaef66275ac00975a259828086be6b9`
- Contents: 2840 resources of 141 resource types, one JSON file each,
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
