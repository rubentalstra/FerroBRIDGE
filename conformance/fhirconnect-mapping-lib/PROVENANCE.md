<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the FHIRconnect mapping library pass list

`pass-list.txt` records which files of the vendored FHIRconnect mapping library
pass, one path relative to `docs/specs/fhirconnect-mapping-lib/` per line, and
the corpus size on its `total` line. It holds no copy of the corpus. The corpus
is the vendored tree, at the pin `docs/VERSIONS.md` records and
`docs/specs/fhirconnect-mapping-lib/PROVENANCE.md` describes.

- Written by: `corpus::conformance_the_fhirconnect_corpus_holds_its_pass_list`
  in `crates/fhirconnect/tests/it/corpus.rs`, under
  `scripts/checks/conformance.sh --update`.
- A case passes when the file parses, validates against the published schemas
  or is in the pinned rejection set, validates against the strict schemas,
  loads into the corpus set with no refusal naming it, and is reached by a
  program the suites compile against a template that carries its archetype.
- Never edit the list by hand.
