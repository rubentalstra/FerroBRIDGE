<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the HL7 v2-to-FHIR benchmark messages

Two paths of the repository, vendored verbatim at their upstream layout by
`scripts/vendor/hl7v2-samples.sh` (.claude/rules/vendored-inputs.md), and
the files the script derives from one of them. Never edit a file here: change
the pin in docs/VERSIONS.md, run the script with `--stamp`, and record the
count and digest it prints.

- Source: <https://github.com/HL7/v2-to-fhir>
- Pin: commit `873b331b3890c8bc5d62ef9b4dabb41801aac70d`
- Paths: `input/pagecontent/test_conversions.md` (the benchmark page) and
  `samples` (HL7-authored R4 Bundles for ADT_A01 and MDM_T02, and the
  MDM_T02 message beside the second)
- Upstream licence: Apache License 2.0, the repository's `LICENSE` file,
  vendored beside this file
- Files: 15, the derived files included
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `dd2831ec1bdef559421849bbe171213e8aa0761eb0105b362c969f4ebc23d8e6`
- Stamped: 2026-09-25

## Derived files

The page embeds each benchmark message in HTML, one segment per line inside a
`<tr>` element. The script writes each into its own file under `derived/`,
named by the `####` heading of its section: the lines between `<tr>` and
`</tr>`, with each line's carriage return and `<br>` opener removed and the
empty lines dropped, one segment per line, bytes otherwise unchanged. These
files modify `input/pagecontent/test_conversions.md`, and this section is the
notice of the change that section 4(b) of the Apache License 2.0 asks for,
kept here because a line in the files would break them as messages:

- `derived/ADT_A01.hl7`
- `derived/MDM_T02.hl7`
- `derived/OML_O21.hl7`
- `derived/ORM_O01.hl7`
- `derived/ORU_R01.hl7`
- `derived/SIU_S12.hl7`
- `derived/VXU_V04.hl7`

The page states no Bundle for any of them ("To be provided"), so the corpus
test reads the HL7-authored `samples/fhir-bundles` Bundles as the
comparison where one exists.
