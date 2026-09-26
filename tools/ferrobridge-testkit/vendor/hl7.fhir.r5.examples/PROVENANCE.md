<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: hl7.fhir.r5.examples

Fetched verbatim at build time by `scripts/vendor/fhir-packages.sh
--build-time` into `package/` beside this file, which `.gitignore` refuses
(.claude/rules/vendored-inputs.md). Never commit a file from there and never
edit one: change the pin in docs/VERSIONS.md, run the script with
`--build-time --stamp`, and record the count and digest it prints.

- Package: hl7.fhir.r5.examples
- Version: 5.0.0
- Source: the FHIR package registry, https://packages.fhir.org/hl7.fhir.r5.examples
- Tarball: https://packages.simplifier.net/hl7.fhir.r5.examples/5.0.0
- SHA-1 (registry shasum): 1565fe6f246c2cc146b18091e82e4848043ad4f1
- SHA-256 (tarball): 129eb96eaeab3b14b4c3a89a2e3593a0a79ebddb56ef46e15fac6928a0a5976e
- Upstream license: CC0-1.0 (the `license` field of `package/package.json`)
- Layout: the tarball's `package/` directory, extracted unchanged
- Files: 2824
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `afddb711d22bbf8c45158cb4be0a8936ecd458df1a2a99cf5debd5274a7bee47`
- Contents: 2822 resources of 157 resource types, one JSON file each,
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
