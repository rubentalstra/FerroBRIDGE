<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the KDS_Diagnose operational template

One file, vendored verbatim by `scripts/vendor/kds-diagnose-opt.sh`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/openFHIR/openfhir>
- Pin: commit `5e4d68007518fcddae1907b333a86f495791aa53`, path `core/src/test/resources/kds/diagnose/KDS_Diagnose.opt`
- File: `KDS_Diagnose.opt`, 167924 bytes, sha256 `752483d90c4ba0f0d1e67baacceb67c1f6f698607823f2eebaa876f4f32bd870`
- Fetched: 2026-09-24
- Upstream licence: Apache License 2.0, the repository's `LICENSE` file at the
  same commit, vendored beside this file. The template itself states
  `<copyright>© HiGHmed</copyright>` and carries no licence element, so its
  content is used on the terms the repository above redistributes it under.
- Use: a test fixture for the FHIR round trip, read by the FerroBRIDGE test
  suites and never shipped in a crate (the testkit crate is `publish = false`).
