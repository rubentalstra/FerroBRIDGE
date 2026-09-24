<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the FHIRconnect specification source

Vendored verbatim by `scripts/vendor/fhirconnect.sh`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/SevKohler/FHIRconnect-spec>
- Pin: commit `195b07fdb4c78da0432fdd1e9dbd127b81be6165`
- Fetched: 2026-09-24
- Upstream licence: Apache License 2.0, the repository's `LICENSE` file,
  vendored beside this file
- Layout: the upstream paths, unchanged
- Files: 74, of which 61 are `modules/ROOT/pages/**/*.adoc`
- Tree digest (sha256 over the sorted per-file `sha256  path` listing, both
  `PROVENANCE.md` files excluded): `f3e9a165a79d2d3cbaf1f9d1516356f21501709fcd2b3bb4eb20d17039da69d1`

## What is here

- `build/site/FHIRconnect/v1.0.0/_attachments/`: the two mapping schemas as the published v1.0.0 site renders
  them. These are the bytes docs/architecture.md section 2 records a sha256 for,
  and the script fails when either disagrees.
- `modules/ROOT/attachments/`: the same two schemas from the repository source at
  the pinned commit. They have moved past the rendered v1.0.0 release:
  `model-mapping.schema.json` gains a `unidirectional` property and
  `contextual-mapping.schema.json` gains an `operational` property.
- `modules/ROOT/pages/` and `modules/ROOT/nav.adoc`: the specification prose
  the design cites, and its navigation.
- `draft-rest-api/`: the unmerged REST API chapter, pinned separately with its
  own `PROVENANCE.md`.

| Schema | sha256 (published v1.0.0) | sha256 (repository source) |
|---|---|---|
| `model-mapping.schema.json` | `6a925151c029e10ef11ccfc2eaafbbf441eea97ffec8ea9e0cd0b8493871d852` | `f92c4902e6b779eecfd40ca4ea774441609db8ffb7163832efc99b562cdfeba4` |
| `contextual-mapping.schema.json` | `a96d600dfa7faacb1b2d8919a20554bed5699a636efd7f903d601b71cae15832` | `0402888b08a1ddfd457eb0e3c3f6c4bdac7f2339762b9553fce4c096c88f25d2` |
