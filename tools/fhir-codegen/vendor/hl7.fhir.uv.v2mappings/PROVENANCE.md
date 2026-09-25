# Provenance: hl7.fhir.uv.v2mappings

Vendored verbatim as codegen input (.claude/rules/vendored-inputs.md). Never
edit a file under `package/`; change the pin in docs/VERSIONS.md and re-run
`scripts/vendor/fhir-packages.sh hl7.fhir.uv.v2mappings`.

- Package: hl7.fhir.uv.v2mappings
- Version: 1.0.0
- Source: the FHIR package registry, https://packages.fhir.org/hl7.fhir.uv.v2mappings
- Tarball: https://packages.simplifier.net/hl7.fhir.uv.v2mappings/1.0.0
- SHA-1 (registry shasum): cf2d2e1af0a627c819830a63c6b0bcdd9599c1c5
- SHA-256 (tarball): 49004948e7589ccc663b3df89f831b3cbf4c2dd1ef67c859633dd50cc81581a0
- Fetched: 2026-09-25
- Upstream license: CC0-1.0 (the `license` field of `package/package.json`)
- Layout: the tarball's `package/` directory, extracted unchanged
- Repository licence: Apache-2.0, the `LICENSE` file of
  <https://github.com/HL7/v2-to-fhir> at tag `1.0.0` (commit `873b331b3890c8bc5d62ef9b4dabb41801aac70d`),
  vendored beside this file as `repository/LICENSE`. The package declares
  CC0-1.0 and the repository it is built from declares Apache-2.0, so both
  statements are here and neither is assumed.
- Repository files: taken at the same commit and checked against the git blob
  id GitHub records for each path

| File | sha256 | git blob id |
|---|---|---|
| `repository/LICENSE` | `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4` | `261eeb9e9f8b2b4b0d119366dda99c6fd7d35c64` |
| `repository/input/pagecontent/mapping_guidelines.md` | `e2b6f95787cd383ef45635fee3854f122e06e2f4199b073fa6c0ea63843c34f1` | `e26193e1d98cb6fa1b66b6974f2396546185470a` |

## Contents

- ConceptMaps: 263 (105 datatype, 13 message, 75 segment, 70 table), counted by the kind their file name opens
  with (`ConceptMap-<kind>-...`)
- Examples: 0 resources of the ImplementationGuide are marked as an
  example (`exampleBoolean` or `exampleCanonical`).
- Pages: the ImplementationGuide names 20 pages, and the package carries
  0 HTML or Markdown files. The "Mapping Guidelines" page (`mapping_guidelines.html`) is
  built from `input/pagecontent/mapping_guidelines.md` of the repository,
  taken beside the package as `repository/input/pagecontent/mapping_guidelines.md`.
- Declared dependencies (`package/package.json`): `hl7.fhir.r4.core` 4.0.1, `hl7.terminology.r4` 6.5.0, `hl7.fhir.uv.extensions.r4` 5.2.0. The terminology
  package this tree vendors is `hl7.terminology` at the version docs/VERSIONS.md
  pins, which is a different package line from the one declared here.
