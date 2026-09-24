#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/its-rest.sh
#
# Vendors the openEHR ITS-REST OpenAPI documents into docs/specs/its-rest/
# (.claude/rules/vendored-inputs.md): the three STABLE code-generation documents
# for the EHR, Query and Definition modules, the AsciiDoc sources of the
# Simplified Formats and Simplified Data Template sub-specifications (the FLAT
# format the composition seam reads and writes), plus the repository's licence
# file.
# Admin and Demographic are `x-status: DEVELOPMENT` in the same release and the
# bridge does not depend on them (docs/architecture.md section 2).
#
# The "openEHR ITS-REST OpenAPI" row of docs/VERSIONS.md pins a tag. A tag is
# mutable and every file in it says `info.version: latest`, so the script
# resolves the tag to a commit and records the commit and the git blob id of
# each file beside it.
#
# Usage:
#   scripts/vendor/its-rest.sh
#
# Requires: curl, tar, shasum, jq.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

corpus_require curl tar shasum jq

dest="docs/specs/its-rest"
oas="computable/OAS"

paths=(
  "$oas/ehr-codegen.openapi.yaml"
  "$oas/query-codegen.openapi.yaml"
  "$oas/definition-codegen.openapi.yaml"
  "docs/simplified_formats"
  "docs/simplified_data_template"
  "LICENSE"
)

pin="$(corpus_pin_cell "openEHR ITS-REST OpenAPI")"
repo="$(corpus_pin_repo "$pin")"
tag="$(corpus_pin_tag "$pin")"
commit="$(corpus_resolve_tag "$repo" "$tag")"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "$repo tag $tag resolves to $commit"
tree_root="$(corpus_fetch "$repo" "$commit" "$tmp")"

rm -rf "$dest"
mkdir -p "$dest"
corpus_take "$tree_root" "$dest" "${paths[@]}"

# Each document must be the STABLE one, and each says `info.version: latest`,
# which is why this provenance records a blob id per file rather than a version.
for path in "${paths[@]}"; do
  case "$path" in
    *.openapi.yaml)
      grep -qE '^[[:space:]]+x-status:[[:space:]]*STABLE$' "$dest/$path" ||
        die "$path is not x-status: STABLE at $tag"
      ;;
    *) ;;
  esac
done
rows=""
while IFS= read -r file; do
  path="${file#"$dest"/}"
  rows="$rows
| \`$path\` | \`$(corpus_sha256 "$file")\` | \`$(corpus_blob_id "$file")\` |"
done < <(find "$dest" -type f ! -name PROVENANCE.md | LC_ALL=C sort)

licence="$(sed -nE 's/^[[:space:]]+name:[[:space:]]*(Creative Commons.*)$/\1/p' \
  "$dest/$oas/ehr-codegen.openapi.yaml" | head -n1)"
[ -n "$licence" ] || die "ehr-codegen.openapi.yaml declares no info.license.name"

files="$(corpus_file_count "$dest")"
digest="$(corpus_tree_digest "$dest")"
fetched="$(corpus_fetched)"

cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the openEHR ITS-REST OpenAPI documents and Simplified Formats

Vendored verbatim by \`scripts/vendor/its-rest.sh\`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/$repo>
- Pin: tag \`$tag\`, which resolves to commit \`$commit\`
- Fetched: $fetched
- Upstream licence: the specification content declares \`$licence\` in each
  document's \`info.license\`. The repository's own \`LICENSE\` file is the
  Apache License 2.0 and is vendored beside this file, so both statements are
  here and neither is assumed.
- Layout: the upstream paths, unchanged
- Files: $files
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`

## Why a blob id per file

Every document at this tag says \`info.version: latest\`, so the file content
carries no release identity of its own. The tag, the commit it resolves to, and
the git blob id of each file are what identify these bytes
(docs/architecture.md section 2).

Each of the three is \`x-status: STABLE\`, which the script checks. The Admin and
Demographic documents of the same release are \`x-status: DEVELOPMENT\` and are
not vendored.

\`docs/simplified_formats/\` and \`docs/simplified_data_template/\` are the
AsciiDoc sources of the Simplified Formats and Simplified Data Template
sub-specifications at the same commit, taken whole. The FLAT format of the
composition seam, its \`ctx/\` shortcuts and its \`_\`-prefixed attribute
families (\`master05-rm_mapping.adoc\`) are specified there.

| File | sha256 | git blob id |
|---|---|---|$rows
PROV

say "$files files, tree digest $digest"
say "done"
