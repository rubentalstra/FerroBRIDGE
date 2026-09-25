<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the Microsoft FHIR-Converter HL7 v2 samples

Two directories of the repository, vendored verbatim at their upstream layout
by `scripts/vendor/hl7v2-samples.sh` (.claude/rules/vendored-inputs.md).
Never edit a file here: change the pin in docs/VERSIONS.md, run the script
with `--stamp`, and record the count and digest it prints.

- Source: <https://github.com/microsoft/FHIR-Converter>
- Pin: commit `70fd328e05019142f616a660cf65c6034baaa3c9`
- Paths: `data/SampleData/Hl7v2` (139 HL7 v2 messages) and
  `src/Microsoft.Health.Fhir.Liquid.Converter.FunctionalTests/TestData/Expected/Hl7v2`
  (132 expected R4 Bundles, one per message it names, under
  `<type>_<event>/<message>-expected.json`)
- Upstream licence: MIT, the repository's `LICENSE` file, vendored beside
  this file
- Files: 272
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `8ba5602d3efc0f1161c1329887e3a9178983e1d2ed76f4353968aacd90c1420c`
- Stamped: 2026-09-25

The messages are test messages the converter's authors wrote, never patient
data. The expected Bundles are what Microsoft's own Liquid templates produce,
so the corpus test reads them as a comparison, never as the v2-to-FHIR
guide's answer: every difference is a counted outcome.
