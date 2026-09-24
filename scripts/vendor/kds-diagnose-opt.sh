#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/kds-diagnose-opt.sh
#
# Vendors the KDS_Diagnose operational template into
# tools/ferrobridge-testkit/fixtures/opt/kds/ (.claude/rules/vendored-inputs.md),
# the one file the "KDS Diagnose operational template (fixture)" row of
# docs/VERSIONS.md pins by repository, commit, path and sha256, plus the
# repository's LICENSE at the same commit.
#
# It is a test fixture for the FHIR round trip, read by the test suites and
# never shipped: the testkit crate is `publish = false`.
#
# Usage:
#   scripts/vendor/kds-diagnose-opt.sh
#
# Requires: curl, shasum, awk.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

corpus_require curl shasum awk

dest="tools/ferrobridge-testkit/fixtures/opt/kds"
item="KDS Diagnose operational template (fixture)"

pin="$(corpus_pin_cell "$item")"
repo="$(corpus_pin_repo "$pin")"
commit="$(corpus_pin_commit "$pin")"
# The pin cell reads `<repo> commit <sha> path <path> sha256 <digest>`.
path="$(awk '{ for (i = 1; i < NF; i++) if ($i == "path") { print $(i + 1); exit } }' <<< "$pin")"
want="$(awk '{ for (i = 1; i < NF; i++) if ($i == "sha256") { print $(i + 1); exit } }' <<< "$pin")"
[ -n "$path" ] || die "the pin '$pin' names no path"
grep -qE '^[0-9a-f]{64}$' <<< "$want" || die "the pin '$pin' names no sha256"

raw="https://raw.githubusercontent.com/$repo/$commit"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "$repo at $commit: $path"
corpus_download "$raw/$path" "$tmp/template.opt" || die "download of $path failed"
corpus_download "$raw/LICENSE" "$tmp/LICENSE" || die "download of LICENSE failed"

got="$(corpus_sha256 "$tmp/template.opt")"
[ "$got" = "$want" ] || die "$path hashes to $got, the pin records $want"
grep -q 'Apache License' "$tmp/LICENSE" || die "the repository LICENSE is not the Apache License"

name="$(basename "$path")"
rm -rf "$dest"
mkdir -p "$dest"
mv "$tmp/template.opt" "$dest/$name"
mv "$tmp/LICENSE" "$dest/LICENSE"

bytes="$(wc -c < "$dest/$name" | tr -d '[:space:]')"
fetched="$(corpus_fetched)"

cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the KDS_Diagnose operational template

One file, vendored verbatim by \`scripts/vendor/kds-diagnose-opt.sh\`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/$repo>
- Pin: commit \`$commit\`, path \`$path\`
- File: \`$name\`, $bytes bytes, sha256 \`$got\`
- Fetched: $fetched
- Upstream licence: Apache License 2.0, the repository's \`LICENSE\` file at the
  same commit, vendored beside this file. The template itself states
  \`<copyright>© HiGHmed</copyright>\` and carries no licence element, so its
  content is used on the terms the repository above redistributes it under.
- Use: a test fixture for the FHIR round trip, read by the FerroBRIDGE test
  suites and never shipped in a crate (the testkit crate is \`publish = false\`).
PROV

say "$name, $bytes bytes, sha256 $got"
say "done"
