<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: hl7.fhir.r6.examples

Fetched verbatim at build time by `scripts/vendor/fhir-packages.sh
--build-time` into `package/` beside this file, which `.gitignore` refuses
(.claude/rules/vendored-inputs.md). Never commit a file from there and never
edit one: change the pin in docs/VERSIONS.md, run the script with
`--build-time --stamp`, and record the count and digest it prints.

- Package: hl7.fhir.r6.examples
- Version: 6.0.0-ballot5
- Source: the FHIR package registry, https://packages2.fhir.org/packages/hl7.fhir.r6.examples
- Tarball: https://packages2.fhir.org/web/hl7.fhir.r6.examples-6.0.0-ballot5.tgz
- SHA-1 (registry shasum): 213e6e7e2193b5973a7c0623936c3afd353948fd
- SHA-256 (tarball): 03e6a7ac71026efd2636560a1fc461d897fe9f6afe318de976b2f2f37d02ba1e
- Upstream license: CC0-1.0 (the `license` field of `package/package.json`)
- Layout: the tarball's `package/` directory, extracted unchanged
- Files: 2488
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `4654c0fa42180b2ef3abec208458482cb1092fe3c63ef4ec4661a3feb50a7b3f`
- Contents: 2486 resources of 126 resource types, one JSON file each,
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
