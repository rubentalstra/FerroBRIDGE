#!/usr/bin/env bash
# SPDX-FileCopyrightText: Ruben Talstra
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/fhirconnect-mapping-lib.sh
#
# Vendors the FHIRconnect mapping library into docs/specs/fhirconnect-mapping-lib/
# (.claude/rules/vendored-inputs.md), the whole repository tree at the commit the
# "FHIRconnect mapping library (corpus, never an oracle)" row of docs/VERSIONS.md
# pins.
#
# It is a conformance corpus, never an oracle: the specification decides what a
# mapping file may contain, and files here are evidence of what real mappings
# look like (docs/architecture.md section 4.8).
#
# Usage:
#   scripts/vendor/fhirconnect-mapping-lib.sh
#
# Requires: curl, tar, shasum, jq.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

corpus_require curl tar shasum jq

dest="docs/specs/fhirconnect-mapping-lib"

pin="$(corpus_pin_cell "FHIRconnect mapping library (corpus, never an oracle)")"
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
digest="$(corpus_tree_digest "$dest")"
fetched="$(corpus_fetched)"

cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the FHIRconnect mapping library

The whole repository tree, vendored verbatim by
\`scripts/vendor/fhirconnect-mapping-lib.sh\`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/$repo>
- Pin: commit \`$commit\`
- Fetched: $fetched
- Upstream licence: Apache License 2.0, the repository's \`LICENSE\` file,
  vendored beside this file
- Layout: the whole upstream tree, unchanged
- Files: $files, of which $yaml are YAML mapping files
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`

This corpus is evidence, never an oracle. The FHIRconnect specification decides
what a mapping file may contain; these files show what published mappings look
like, including where they disagree with the published schema
(docs/architecture.md section 4.8).
PROV

say "$files files, $yaml YAML, tree digest $digest"
say "done"
