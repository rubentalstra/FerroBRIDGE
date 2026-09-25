#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/v2ig.sh
#
# Fetches the HL7 v2 definitions the generator reads, the `input/sourceOfTruth`
# tree of HL7/v2ig, into tools/fhir-codegen/vendor/hl7-v2ig/ at build time.
# The repository carries no licence that permits redistribution, so the tree is
# never committed: .gitignore refuses everything in that directory except its
# PROVENANCE.md (.claude/rules/vendored-inputs.md, Licensing).
#
# The "HL7 v2 definitions" row of docs/VERSIONS.md pins the commit, the file
# count and the tree digest. Every run verifies the tree against both, and a
# tree already on disk at the pinned digest (a CI cache) is kept without a
# download.
#
# Usage:
#   scripts/vendor/v2ig.sh            # fetch or verify at the pin
#   scripts/vendor/v2ig.sh --commit   # print the pinned commit (a CI cache key)
#   scripts/vendor/v2ig.sh --stamp    # after moving the pin: fetch, rewrite the
#                                     # committed PROVENANCE.md, print the count
#                                     # and digest for the docs/VERSIONS.md row
#
# Requires: curl, tar, shasum, jq, git.
#
# No specification governs this script; it is FerroBRIDGE's own design.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

mode="${1:-fetch}"
case "$mode" in
  fetch | --commit | --stamp) ;;
  *) die "usage: scripts/vendor/v2ig.sh [--commit | --stamp]" ;;
esac

corpus_require curl tar shasum jq git

dest="tools/fhir-codegen/vendor/hl7-v2ig"
item="HL7 v2 definitions (v2ig source of truth, never committed)"
licence_item="HL7 v2+ licence page"

# The token after the word $1 in the pin cell $2.
pin_field() {
  awk -v key="$1" '{ for (i = 1; i < NF; i++) if ($i == key) { print $(i + 1); exit } }' <<< "$2"
}

pin="$(corpus_pin_cell "$item")"
repo="$(corpus_pin_repo "$pin")"
commit="$(corpus_pin_commit "$pin")"
path="$(pin_field path "$pin")"
want_files="$(pin_field files "$pin")"
want_digest="$(pin_field digest "$pin")"
[ -n "$path" ] || die "the pin '$pin' names no path"

if [ "$mode" = "--commit" ]; then
  printf '%s\n' "$commit"
  exit 0
fi

if [ "$mode" = "fetch" ]; then
  grep -qE '^[0-9]+$' <<< "$want_files" || die "the pin '$pin' names no file count"
  grep -qE '^[0-9a-f]{64}$' <<< "$want_digest" || die "the pin '$pin' names no tree digest"
fi

# The file count and tree digest of the tree under $1, without PROVENANCE.md.
tree_state() {
  if [ -d "$1/$path" ]; then
    printf '%s %s\n' "$(corpus_file_count "$1")" "$(corpus_tree_digest "$1")"
  fi
}

if [ "$mode" = "fetch" ] && [ "$(tree_state "$dest")" = "$want_files $want_digest" ]; then
  say "$dest already holds $repo $path at $commit ($want_files files, digest $want_digest)"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "$repo at $commit: $path"
tree_root="$(corpus_fetch "$repo" "$commit" "$tmp")"
# The provenance records that the repository states no licence; a LICENSE that
# appears at a new pin has to be read before the tree is used.
if find "$tree_root" -maxdepth 1 -iname 'licen[cs]e*' | grep -q .; then
  die "$repo at $commit carries a licence file; read it and revise the provenance before using this pin"
fi
readme_line="$(grep -m1 -F 'not yet production ready' "$tree_root/README.md" || true)"

# The tree is staged and verified first, so a mismatch leaves the destination
# as it was.
mkdir -p "$tmp/stage"
corpus_take "$tree_root" "$tmp/stage" "$path"
read -r files digest <<< "$(tree_state "$tmp/stage")"
if [ "$mode" = "fetch" ]; then
  [ "$files" = "$want_files" ] || die "the tree holds $files files, the pin records $want_files"
  [ "$digest" = "$want_digest" ] || die "the tree digest is $digest, the pin records $want_digest"
  grep -qF "$commit" "$dest/PROVENANCE.md" || die "$dest/PROVENANCE.md does not name commit $commit"
  grep -qF "$digest" "$dest/PROVENANCE.md" || die "$dest/PROVENANCE.md does not name digest $digest"
fi

# Everything in the destination except the committed PROVENANCE.md is replaced.
mkdir -p "$dest"
find "$dest" -mindepth 1 -maxdepth 1 ! -name PROVENANCE.md -exec rm -rf {} +
mkdir -p "$dest/$(dirname "$path")"
mv "$tmp/stage/$path" "$dest/$path"

git check-ignore -q "$dest/$path" ||
  die "$dest/$path is not ignored by git; the tree must never be committed"
[ -z "$(git ls-files -- "$dest" ":(exclude)$dest/PROVENANCE.md")" ] ||
  die "git tracks files under $dest other than PROVENANCE.md"
[ "$(tree_state "$dest")" = "$files $digest" ] || die "the tree moved into $dest does not match the staged one"

if [ "$mode" = "fetch" ]; then
  say "$files files, tree digest $digest"
  say "done"
  exit 0
fi

# --stamp: read the licence page HL7 attaches to its v2+ publication at its
# pinned commit and quote its passages verbatim, line by line.
lpin="$(corpus_pin_cell "$licence_item")"
lrepo="$(corpus_pin_repo "$lpin")"
lcommit="$(corpus_pin_commit "$lpin")"
lpath="$(pin_field path "$lpin")"
lsha="$(pin_field sha256 "$lpin")"
corpus_download "https://raw.githubusercontent.com/$lrepo/$lcommit/$lpath" "$tmp/licence.html" ||
  die "download of $lpath from $lrepo failed"
got="$(corpus_sha256 "$tmp/licence.html")"
[ "$got" = "$lsha" ] || die "$lpath hashes to $got, the pin records $lsha"

quotes=""
for opening in \
  "The 2019 Health Level Seven v2+ Publication is copyrighted" \
  "HL7 licenses its standards and select IP free of charge." \
  "A. HL7 INDIVIDUAL, STUDENT AND HEALTH PROFESSIONAL MEMBERS, who register" \
  "INDIVIDUAL, STUDENT AND HEALTH PROFESSIONAL MEMBERS wishing to incorporate" \
  "B. HL7 ORGANIZATION MEMBERS, who register" \
  "C. NON-MEMBERS, who register" \
  "NON-MEMBERS wishing to incorporate"; do
  line="$(grep -F "$opening" "$tmp/licence.html")" || die "$lpath has no line opening '$opening'"
  [ "$(wc -l <<< "$line" | tr -d '[:space:]')" = "1" ] || die "$lpath has more than one line opening '$opening'"
  quotes="$quotes
> $line
>"
done
quotes="${quotes%>}"
fetched="$(corpus_fetched)"

cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: the HL7 v2 definitions (HL7/v2ig source of truth)

Fetched verbatim at build time by \`scripts/vendor/v2ig.sh\` into this
directory, which \`.gitignore\` refuses except for this file. Never commit a
file from here and never edit one: change the pin in docs/VERSIONS.md, run
\`scripts/vendor/v2ig.sh --stamp\`, and record the count and digest it prints.

- Source: <https://github.com/$repo>
- Pin: commit \`$commit\`, path \`$path\`, taken whole at its upstream layout
- Files: $files
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`
- Stamped: $fetched. The script verifies the count and the digest on every
  fetch, and keeps a tree already on disk at that digest.
- Content: FHIR StructureDefinitions of kind \`logical\` (with a few
  CodeSystem, ValueSet and List resources) for the v2 messages, message
  structures, events, segments and data types, declaring \`fhirVersion\`
  5.0.0. They hold one v2 version: the repository's
  \`V291_EXTRACTION_SUMMARY.md\` describes their extraction from the HL7
  V2.9.1 Word documents.
- Use: generator input for \`tools/fhir-codegen\` and nothing else. The tree is
  never packaged and never published.

## Licence

The repository states no licence. It has no \`LICENSE\` file at this commit
(the script refuses a pin where one appears), and GitHub reports no licence
for it. Its README reads:

> $readme_line

The licence text HL7 attaches to its machine-readable v2 publication is the
licence page of the HL7 v2+ site, <https://github.com/$lrepo> commit
\`$lcommit\`, \`$lpath\` (sha256 \`$lsha\`). It names the 2019 v2+
publication and the \`http://www.HL7.org/legal/ippolicy.cfm\` terms. Its
passages on use, quoted verbatim, one source line each:
$quotes
Copying for internal purposes only does not permit redistribution, so this
tree takes the path for such material: it is fetched into an ignored
directory at build time and the repository ships none of it.

## The owner's decision (2026-09-25)

The owner decided on 2026-09-25 to use this material as a generator input,
under the HL7 organisational membership the owner holds, which is the
condition the licence text above sets on incorporating the specified material.
The Rust code the generator derives from it is the project's own code and
carries Apache-2.0, as \`fhir-types\` does for the FHIR model (#253). The
fetched definitions themselves are never committed, packaged or published.
PROV

say "$files files, tree digest $digest"
say "record in the docs/VERSIONS.md row: files $files digest $digest"
say "done"
