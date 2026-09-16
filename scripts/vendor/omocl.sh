#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/omocl.sh
#
# Vendors the OMOCL corpus into docs/specs/omocl/
# (.claude/rules/vendored-inputs.md), the whole repository tree at the commit the
# "OMOCL corpus" row of docs/VERSIONS.md pins.
#
# The pin is a commit and not the git tag `v1.0.0`, because that tag carries
# pre-grammar files: every mapping file at this commit opens with
# `grammar: OMOCL/v1.0.0`, which the script verifies (docs/architecture.md
# section 2).
#
# Usage:
#   scripts/vendor/omocl.sh
#
# Requires: curl, tar, shasum, jq.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

corpus_require curl tar shasum jq

dest="docs/specs/omocl"
grammar="OMOCL/v1.0.0"

pin="$(corpus_pin_cell "OMOCL corpus")"
repo="$(corpus_pin_repo "$pin")"
commit="$(corpus_pin_commit "$pin")"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "$repo at $commit"
tree_root="$(corpus_fetch "$repo" "$commit" "$tmp")"
[ -f "$tree_root/LICENSE" ] || die "the archive has no LICENSE"

rm -rf "$dest"
mkdir -p "$(dirname "$dest")"
mv "$tree_root" "$dest"

files="$(corpus_file_count "$dest")"
yaml="$(find "$dest" -type f \( -name '*.yml' -o -name '*.yaml' \) | wc -l | tr -d '[:space:]')"
# The corpus ships with CRLF line endings, so the grammar line ends in a
# carriage return; `.gitattributes` marks the tree `-text` to keep those bytes.
carrying="$(grep -rlE "^grammar:[[:space:]]*${grammar}[[:space:]]*\$" "$dest" | wc -l | tr -d '[:space:]')"
[ "$carrying" = "$yaml" ] ||
  die "$carrying of $yaml mapping files declare '$grammar'; the pinned commit must carry the released grammar"
digest="$(corpus_tree_digest "$dest")"
fetched="$(corpus_fetched)"

cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the OMOCL corpus

The whole repository tree, vendored verbatim by \`scripts/vendor/omocl.sh\`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/$repo>
- Pin: commit \`$commit\`
- Fetched: $fetched
- Upstream licence: Apache License 2.0, the repository's \`LICENSE\` file,
  vendored beside this file
- Layout: the whole upstream tree, unchanged
- Line endings: CRLF, as upstream ships them. \`.gitattributes\` marks this tree
  \`-text\` so git stores those bytes unchanged.
- Files: $files, of which $yaml are mapping files, and all $carrying of those
  declare \`grammar: $grammar\`
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`

## Why the pin is a commit

The git tag \`v1.0.0\` of this repository is not this commit. That tag carries
pre-grammar files headed \`engine: EOS/v0.0.62\`, and the grammar string
\`$grammar\` first appears at tag \`v1.0.1\`. Pinning the commit is what makes
the corpus the released grammar (docs/architecture.md section 2).
PROV

say "$files files, $yaml mapping files on $grammar, tree digest $digest"
say "done"
