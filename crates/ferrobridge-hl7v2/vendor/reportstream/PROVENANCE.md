<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the CDC ReportStream data tests

One directory of the repository, vendored verbatim at its upstream layout by
`scripts/vendor/hl7v2-samples.sh` (.claude/rules/vendored-inputs.md). Never
edit a file here: change the pin in docs/VERSIONS.md, run the script with
`--stamp`, and record the count and digest it prints.

- Source: <https://github.com/CDCgov/prime-reportstream> (archived by its owner)
- Pin: commit `5f58b4c574fa64a70b362f7881f035ec6110a933`
- Path: `prime-router/src/testIntegration/resources/datatests`: 364
  `.hl7` files (single messages and `FHS`/`BHS` batches) and 329
  `.fhir` R4 Bundles, with the CSV inputs and the test configuration
- Upstream licence: CC0-1.0, the repository's `LICENSE` file, vendored
  beside this file
- Files: 732
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `857645f53e03efed22325254f1757773c3812f58f3b9b13f8cda796ba5415f3c`
- Stamped: 2026-09-25

The messages are test messages ReportStream's authors wrote, never patient
data. In `HL7_to_FHIR/` and `mappinginventory/` a `.fhir` Bundle beside a
`.hl7` message is ReportStream's conversion of it, which the corpus test
reads as a comparison, never as the v2-to-FHIR guide's answer. In
`FHIR_to_HL7/` the direction is the other way, so no Bundle there is read as
an expected result.
