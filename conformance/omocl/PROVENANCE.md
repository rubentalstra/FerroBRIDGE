<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the OMOCL mapping library pass list

`pass-list.txt` records which files of the vendored OMOCL corpus pass, one path
relative to `docs/specs/omocl/` per line, and the corpus size on its `total`
line. It holds no copy of the corpus. The corpus is the vendored tree, at the
pin `docs/VERSIONS.md` records and `docs/specs/omocl/PROVENANCE.md` describes.

- Written by: `corpus::conformance_the_omocl_corpus_holds_its_pass_list` in
  `crates/omocl/tests/it/corpus.rs`, under
  `scripts/checks/conformance.sh --update`.
- A case passes when the file parses, validates against the authored schema,
  passes the rules of one file, and loads into the corpus set with every
  `Include` it writes resolved.
- Never edit the list by hand.
