<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the OMOCL corpus

The whole repository tree, vendored verbatim by `scripts/vendor/omocl.sh`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/SevKohler/OMOCL>
- Pin: commit `dd42574fdb074c02cbe077a0c49b1bb5bae28f35`
- Fetched: 2026-09-12
- Upstream licence: Apache License 2.0, the repository's `LICENSE` file,
  vendored beside this file
- Layout: the whole upstream tree, unchanged
- Line endings: CRLF, as upstream ships them. `.gitattributes` marks this tree
  `-text` so git stores those bytes unchanged.
- Files: 209, of which 202 are mapping files, and all 202 of those
  declare `grammar: OMOCL/v1.0.0`
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `eec6df1038cd217e3f1ed0d7a717c60d88d9855974354bca5850506350ad2ef4`

## Why the pin is a commit

The git tag `v1.0.0` of this repository is not this commit. That tag carries
pre-grammar files headed `engine: EOS/v0.0.62`, and the grammar string
`OMOCL/v1.0.0` first appears at tag `v1.0.1`. Pinning the commit is what makes
the corpus the released grammar (docs/architecture.md section 2).
