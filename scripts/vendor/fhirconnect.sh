#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/fhirconnect.sh
#
# Vendors the FHIRconnect specification source into docs/specs/fhirconnect/
# (.claude/rules/vendored-inputs.md): the two published draft-07 mapping
# schemas, the prose pages the design cites, the navigation, and the upstream
# licence, from the commit the "FHIRconnect specification source" row of
# docs/VERSIONS.md pins. It then vendors the unmerged REST API chapter and its
# FSH operation definitions into docs/specs/fhirconnect/draft-rest-api/ from the
# commit the "FHIRconnect REST API chapter (draft, unmerged)" row pins.
#
# The schemas ship twice upstream, and the two copies differ at the pinned
# commit: build/site/FHIRconnect/v1.0.0/_attachments/ is the rendered v1.0.0
# release, and modules/ROOT/attachments/ is the working source, which has moved
# past it. The sha256 values docs/architecture.md section 2 records are the
# rendered ones, so this script verifies those and records both.
#
# Usage:
#   scripts/vendor/fhirconnect.sh
#
# Requires: curl, tar, shasum, jq.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

corpus_require curl tar shasum jq

dest="docs/specs/fhirconnect"
draft_dest="$dest/draft-rest-api"
architecture="docs/architecture.md"

published="build/site/FHIRconnect/v1.0.0/_attachments"
source_attachments="modules/ROOT/attachments"

# The sha256 the architecture's pin table records for a rendered v1.0.0 schema.
recorded_sha256() {
  local file="$1" pattern sha
  pattern="$file"'. sha256 .[0-9a-f]{64}'
  sha="$(grep -oE "$pattern" "$architecture" | grep -oE '[0-9a-f]{64}' | head -n1)"
  [ -n "$sha" ] || die "$architecture records no sha256 for $file"
  printf '%s\n' "$sha"
}

spec_pin="$(corpus_pin_cell "FHIRconnect specification source")"
spec_repo="$(corpus_pin_repo "$spec_pin")"
spec_commit="$(corpus_pin_commit "$spec_pin")"

draft_pin="$(corpus_pin_cell "FHIRconnect REST API chapter (draft, unmerged)")"
draft_repo="$(corpus_pin_repo "$draft_pin")"
draft_commit="$(corpus_pin_commit "$draft_pin")"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "specification source $spec_repo at $spec_commit"
mkdir -p "$tmp/spec"
spec_root="$(corpus_fetch "$spec_repo" "$spec_commit" "$tmp/spec")"

rm -rf "$dest"
mkdir -p "$dest"
corpus_take "$spec_root" "$dest" \
  "$published/model-mapping.schema.json" \
  "$published/contextual-mapping.schema.json" \
  "$source_attachments/model-mapping.schema.json" \
  "$source_attachments/contextual-mapping.schema.json" \
  "modules/ROOT/nav.adoc" \
  "modules/ROOT/pages" \
  "LICENSE"

# The pages tree is the cited prose; a non-adoc file appearing there would be
# content this corpus does not describe, so refuse it rather than vendor it.
stray="$(find "$dest/modules/ROOT/pages" -type f ! -name '*.adoc' | head -n1)"
[ -z "$stray" ] || die "modules/ROOT/pages carries a non-adoc file: ${stray#"$dest/"}"
pages="$(find "$dest/modules/ROOT/pages" -type f -name '*.adoc' | wc -l | tr -d '[:space:]')"

declare -a schema_rows=()
for schema in model-mapping contextual-mapping; do
  want="$(recorded_sha256 "$schema.schema.json")"
  got="$(corpus_sha256 "$dest/$published/$schema.schema.json")"
  [ "$got" = "$want" ] || die "$schema.schema.json: $architecture records $want, the rendered v1.0.0 file hashes $got"
  src_sha="$(corpus_sha256 "$dest/$source_attachments/$schema.schema.json")"
  schema_rows+=("| \`$schema.schema.json\` | \`$got\` | \`$src_sha\` |")
  say "$schema.schema.json rendered $got (matches $architecture), source $src_sha"
done

say "draft REST API chapter $draft_repo at $draft_commit"
mkdir -p "$tmp/draft"
draft_root="$(corpus_fetch "$draft_repo" "$draft_commit" "$tmp/draft")"
mkdir -p "$draft_dest"
corpus_take "$draft_root" "$draft_dest" \
  "modules/ROOT/pages/engine/rest-api.adoc" \
  "rest/input/fsh/operations/ToFhir.fsh" \
  "rest/input/fsh/operations/ToOpenEhr.fsh" \
  "rest/input/pagecontent/operations.md"

fetched="$(corpus_fetched)"

draft_rows=""
while IFS= read -r file; do
  draft_rows="$draft_rows
| \`$file\` | \`$(corpus_sha256 "$draft_dest/$file")\` |"
done < <(cd "$draft_dest" && find . -type f ! -name PROVENANCE.md | sed 's|^\./||' | LC_ALL=C sort)

cat > "$draft_dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the FHIRconnect REST API chapter (draft, unmerged)

An **unmerged draft**, pinned by commit. It is pull request #93 of the
FHIRconnect specification, not part of any release, and the bridge implements it
as a draft and re-adjudicates on merge (docs/architecture.md section 4.7).
Vendored verbatim by \`scripts/vendor/fhirconnect.sh\`; never edit a file here.

- Source: <https://github.com/$draft_repo/pull/93>
- Pin: pull request #93 at head commit \`$draft_commit\`
- Fetched: $fetched
- Upstream licence: Apache License 2.0, the \`LICENSE\` file of the same
  repository, vendored at \`../LICENSE\`
- Layout: the upstream paths, unchanged

| File | sha256 |
|---|---|$draft_rows
PROV

files="$(corpus_file_count "$dest")"
digest="$(corpus_tree_digest "$dest")"

cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the FHIRconnect specification source

Vendored verbatim by \`scripts/vendor/fhirconnect.sh\`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/$spec_repo>
- Pin: commit \`$spec_commit\`
- Fetched: $fetched
- Upstream licence: Apache License 2.0, the repository's \`LICENSE\` file,
  vendored beside this file
- Layout: the upstream paths, unchanged
- Files: $files, of which $pages are \`modules/ROOT/pages/**/*.adoc\`
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing, both
  \`PROVENANCE.md\` files excluded): \`$digest\`

## What is here

- \`$published/\`: the two mapping schemas as the published v1.0.0 site renders
  them. These are the bytes docs/architecture.md section 2 records a sha256 for,
  and the script fails when either disagrees.
- \`$source_attachments/\`: the same two schemas from the repository source at
  the pinned commit. They have moved past the rendered v1.0.0 release:
  \`model-mapping.schema.json\` gains a \`unidirectional\` property and
  \`contextual-mapping.schema.json\` gains an \`operational\` property.
- \`modules/ROOT/pages/\` and \`modules/ROOT/nav.adoc\`: the specification prose
  the design cites, and its navigation.
- \`draft-rest-api/\`: the unmerged REST API chapter, pinned separately with its
  own \`PROVENANCE.md\`.

| Schema | sha256 (published v1.0.0) | sha256 (repository source) |
|---|---|---|
$(printf '%s\n' "${schema_rows[@]}")
PROV

say "$files files, $pages pages, tree digest $digest"
say "done"
