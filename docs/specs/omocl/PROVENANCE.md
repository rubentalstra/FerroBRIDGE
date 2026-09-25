<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the OMOCL corpus

The whole repository tree, vendored verbatim by `scripts/vendor/omocl.sh`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/SevKohler/OMOCL>
- Pin: commit `c082db8ed81a062a574a2c058366045f600c2ed7`
- Fetched: 2026-09-25
- Upstream licence: Apache License 2.0, the repository's `LICENSE` file,
  vendored beside this file
- Layout: the whole upstream tree, unchanged
- Line endings: CRLF, as upstream ships them. `.gitattributes` marks this tree
  `-text` so git stores those bytes unchanged.
- Files: 215, of which 208 are mapping files, and all 208 of those
  declare `grammar: OMOCL/v1.0.0`
- Tree digest (sha256 over the sorted per-file `sha256  path` listing,
  `PROVENANCE.md` excluded): `0b0a0350c4f971e243f96b4dacd559ba73e82f8275235c3942d421133c501309`

## Why the pin is a commit

The git tag `v1.0.0` of this repository is not this commit. That tag carries
pre-grammar files headed `engine: EOS/v0.0.62`, and the grammar string
`OMOCL/v1.0.0` first appears at tag `v1.0.1`. Pinning the commit is what makes
the corpus the released grammar (docs/architecture.md section 2).
