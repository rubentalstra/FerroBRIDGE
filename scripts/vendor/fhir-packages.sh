#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/fhir-packages.sh
#
# Vendors the pinned HL7 FHIR packages the generator consumes
# (.claude/rules/vendored-inputs.md). For each package it reads the pin from
# the FHIR table in docs/VERSIONS.md (the single source of truth), downloads
# that exact version from the FHIR package registry, verifies the tarball
# against the registry's recorded checksum, extracts it verbatim into
# tools/fhir-codegen/vendor/<package>/package/, and writes a
# PROVENANCE.md beside it. Re-running with unchanged pins reproduces the same
# tree byte for byte (only the fetch date in PROVENANCE.md moves).
#
# A package whose source repository states a licence of its own also takes,
# at the repository tag its version names, that repository's LICENSE and the
# pages the package names without carrying, into <package>/repository/, so its
# PROVENANCE.md records both licences and assumes neither.
#
# The four examples packages (#373) are the corpora of the fhir-types and
# facade conformance tests. At about 600 MB unpacked, beside the 380 MB above,
# they are fetched at build time into the ignored
# tools/ferrobridge-testkit/vendor/<package>/package/, and only each
# PROVENANCE.md is committed. Their pins carry the file count and tree digest
# a fetch is checked against, and a tree already on disk at both is kept.
#
# Usage:
#   scripts/vendor/fhir-packages.sh                # the default set below
#   scripts/vendor/fhir-packages.sh hl7.terminology  # one or more packages
#   scripts/vendor/fhir-packages.sh --build-time   # verify or fetch the examples packages
#   scripts/vendor/fhir-packages.sh --build-time --stamp
#                                                  # after moving a pin: fetch, rewrite each
#                                                  # PROVENANCE.md, print the count and digest
#   scripts/vendor/fhir-packages.sh --cache-key    # a CI cache key over the examples pins
#
# Requires: curl, jq, tar, shasum, and git for --build-time.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

registry="https://packages.fhir.org"
registry2="https://packages2.fhir.org/packages"
vendor_dir="tools/fhir-codegen/vendor"
pins="docs/VERSIONS.md"
default_packages=(hl7.fhir.r4.core hl7.fhir.r4b.core hl7.fhir.r5.core hl7.fhir.r6.core hl7.terminology hl7.fhir.uv.v2mappings)
examples_dir="tools/ferrobridge-testkit/vendor"
example_packages=(hl7.fhir.r4.examples hl7.fhir.r4b.examples hl7.fhir.r5.examples hl7.fhir.r6.examples)
ua="ferrobridge-vendor (scripts/vendor/fhir-packages.sh)"

# The source repository of a package, where that repository states a licence
# of its own, and the paths taken from it. Each such repository tags every
# published package version with the version itself.
repository_for() {
  case "$1" in
    hl7.fhir.uv.v2mappings) printf '%s\n' "HL7/v2-to-fhir" ;;
    *) ;;
  esac
}

repository_paths_for() {
  case "$1" in
    hl7.fhir.uv.v2mappings) printf '%s\n' LICENSE input/pagecontent/mapping_guidelines.md ;;
    *) ;;
  esac
}

for tool in curl jq tar shasum; do
  command -v "$tool" >/dev/null 2>&1 || die "missing required tool: $tool"
done

# The registry metadata URL of a package. The R6 ballots are published on
# packages2.fhir.org only; every other package comes from packages.fhir.org.
# Both speak the npm registry shape.
meta_url_for() {
  case "$1" in
    hl7.fhir.r6.*) printf '%s\n' "$registry2/$1" ;;
    *) printf '%s\n' "$registry/$1" ;;
  esac
}

# The pin for a package: the second cell of its row in the docs/VERSIONS.md
# FHIR table, e.g. "| `hl7.terminology` (THO) | 7.3.0 | ... |".
pin_for() {
  local pkg="$1"
  awk -F'|' -v pkg="$pkg" '
    $2 ~ "^[[:space:]]*`" pkg "`" {
      v = $3; gsub(/^[[:space:]]+|[[:space:]]+$/, "", v); print v; exit
    }' "$pins"
}

# The tally the v2-to-FHIR provenance records: the ConceptMaps by the kind
# their file name opens with, the IG's pages, its examples, and the packages it
# depends on, read from the extracted package directory $1.
v2mappings_contents() {
  local dir="$1" ig total kinds examples pages texts guidelines deps
  ig="$dir/ImplementationGuide-hl7.fhir.uv.v2mappings.json"
  [[ -f "$ig" ]] || die "hl7.fhir.uv.v2mappings has no ImplementationGuide resource"
  total="$(find "$dir" -maxdepth 1 -name 'ConceptMap-*.json' | wc -l | tr -d '[:space:]')"
  [[ "$total" -gt 0 ]] || die "hl7.fhir.uv.v2mappings carries no ConceptMap"
  kinds="$(find "$dir" -maxdepth 1 -name 'ConceptMap-*.json' -exec basename {} \; |
    sed -E 's/^ConceptMap-([a-z]+)-.*/\1/' | LC_ALL=C sort | uniq -c |
    awk '{ printf "%s%s %s", (NR > 1 ? ", " : ""), $1, $2 }')"
  examples="$(jq '[.definition.resource[] | select(.exampleBoolean == true or .exampleCanonical != null)] | length' "$ig")"
  pages="$(jq '[.definition.page | .. | .nameUrl? // empty] | length' "$ig")"
  texts="$(find "$dir" -type f \( -name '*.html' -o -name '*.md' \) | wc -l | tr -d '[:space:]')"
  guidelines="$(jq -r '[.definition.page | .. | select(.nameUrl? == "mapping_guidelines.html") | .title] | first // empty' "$ig")"
  [[ -n "$guidelines" ]] || die "the IG names no mapping_guidelines.html page"
  deps="$(jq -r '.dependencies // {} | to_entries | map("`\(.key)` \(.value)") | join(", ")' "$dir/package.json")"
  cat <<CONTENTS
## Contents

- ConceptMaps: $total ($kinds), counted by the kind their file name opens
  with (\`ConceptMap-<kind>-...\`)
- Examples: $examples resources of the ImplementationGuide are marked as an
  example (\`exampleBoolean\` or \`exampleCanonical\`).
- Pages: the ImplementationGuide names $pages pages, and the package carries
  $texts HTML or Markdown files. The "$guidelines" page (\`mapping_guidelines.html\`) is
  built from \`input/pagecontent/mapping_guidelines.md\` of the repository,
  taken beside the package as \`repository/input/pagecontent/mapping_guidelines.md\`.
- Declared dependencies (\`package/package.json\`): $deps. The terminology
  package this tree vendors is \`hl7.terminology\` at the version docs/VERSIONS.md
  pins, which is a different package line from the one declared here.
CONTENTS
}

vendor_one() {
  local pkg="$1" ver dest meta shasum tarball_url tmp got sha256 license fetched
  ver="$(pin_for "$pkg")"
  [[ -n "$ver" ]] || die "no pin for $pkg in $pins (add a row to the FHIR table)"
  case "$ver" in
    *[!A-Za-z0-9.-]*) die "pin for $pkg is not a plain version: '$ver'" ;;
  esac
  dest="$vendor_dir/$pkg"

  echo "== $pkg $ver"
  local meta_url
  meta_url="$(meta_url_for "$pkg")"
  meta="$(curl --proto '=https' --tlsv1.2 -fsSL -A "$ua" "$meta_url")" \
    || die "cannot read registry metadata for $pkg"
  shasum="$(printf '%s' "$meta" | jq -r --arg v "$ver" '.versions[$v].dist.shasum // empty')"
  tarball_url="$(printf '%s' "$meta" | jq -r --arg v "$ver" '.versions[$v].dist.tarball // empty')"
  [[ -n "$shasum" ]] || die "the registry lists no version $ver of $pkg"

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  [[ -n "$tarball_url" ]] || die "the registry lists no tarball for $pkg $ver"
  curl --proto '=https' --tlsv1.2 -fsSL -A "$ua" -o "$tmp/pkg.tgz" "$tarball_url" \
    || die "download of $pkg $ver failed"
  got="$(shasum -a 1 "$tmp/pkg.tgz" | cut -d' ' -f1)"
  [[ "$got" = "$shasum" ]] || die "checksum mismatch for $pkg $ver: registry $shasum, downloaded $got"
  sha256="$(shasum -a 256 "$tmp/pkg.tgz" | cut -d' ' -f1)"

  mkdir -p "$tmp/x"
  tar -xzf "$tmp/pkg.tgz" -C "$tmp/x"
  [[ -f "$tmp/x/package/package.json" ]] || die "$pkg $ver has no package/package.json"
  # A FHIR package never carries SNOMED CT release files; refuse anything that
  # looks like RF2 before it can reach the tree (.claude/rules/vendored-inputs.md).
  if find "$tmp/x" -type f \( -name 'sct2_*' -o -name 'der2_*' -o -name '*.rf2' \) | grep -q .; then
    die "$pkg $ver contains RF2-shaped files; SNOMED CT content is never vendored"
  fi
  license="$(jq -r '.license // "unstated"' "$tmp/x/package/package.json")"
  fetched="$(date -u +%Y-%m-%d)"

  local repo repo_commit repo_licence repo_block="" repo_rows="" path want_blob got_blob
  repo="$(repository_for "$pkg")"
  if [[ -n "$repo" ]]; then
    repo_commit="$(corpus_resolve_tag "$repo" "$ver")"
    repo_licence="$(corpus_api "https://api.github.com/repos/$repo/license?ref=$repo_commit" |
      jq -r '.license.spdx_id // empty')" || die "cannot read the licence of $repo at $repo_commit"
    [[ -n "$repo_licence" ]] || die "GitHub reports no licence for $repo at $repo_commit"
    while IFS= read -r path; do
      mkdir -p "$tmp/repo/$(dirname "$path")"
      corpus_download "https://raw.githubusercontent.com/$repo/$repo_commit/$path" "$tmp/repo/$path" ||
        die "download of $path from $repo at $repo_commit failed"
      # The blob id GitHub records for the path at that commit proves the bytes.
      want_blob="$(corpus_api "https://api.github.com/repos/$repo/contents/$path?ref=$repo_commit" |
        jq -r '.sha // empty')" || die "cannot read the blob id of $path in $repo"
      got_blob="$(corpus_blob_id "$tmp/repo/$path")"
      [[ "$got_blob" = "$want_blob" ]] || die "$path of $repo at $repo_commit: blob $got_blob, GitHub records $want_blob"
      repo_rows="$repo_rows
| \`repository/$path\` | \`$(corpus_sha256 "$tmp/repo/$path")\` | \`$got_blob\` |"
    done < <(repository_paths_for "$pkg")
    [[ -f "$tmp/repo/LICENSE" ]] || die "no LICENSE is taken from $repo"
    repo_block="- Repository licence: $repo_licence, the \`LICENSE\` file of
  <https://github.com/$repo> at tag \`$ver\` (commit \`$repo_commit\`),
  vendored beside this file as \`repository/LICENSE\`. The package declares
  $license and the repository it is built from declares $repo_licence, so both
  statements are here and neither is assumed.
- Repository files: taken at the same commit and checked against the git blob
  id GitHub records for each path

| File | sha256 | git blob id |
|---|---|---|$repo_rows"
  fi

  local contents_block=""
  case "$pkg" in
    hl7.fhir.uv.v2mappings) contents_block="$(v2mappings_contents "$tmp/x/package")" ;;
    *) ;;
  esac

  rm -rf "$dest"
  mkdir -p "$dest"
  mv "$tmp/x/package" "$dest/package"
  if [[ -n "$repo" ]]; then
    mv "$tmp/repo" "$dest/repository"
  fi
  cat > "$dest/PROVENANCE.md" <<PROV
# Provenance: $pkg

Vendored verbatim as codegen input (.claude/rules/vendored-inputs.md). Never
edit a file under \`package/\`; change the pin in docs/VERSIONS.md and re-run
\`scripts/vendor/fhir-packages.sh $pkg\`.

- Package: $pkg
- Version: $ver
- Source: the FHIR package registry, $meta_url
- Tarball: $tarball_url
- SHA-1 (registry shasum): $shasum
- SHA-256 (tarball): $sha256
- Fetched: $fetched
- Upstream license: $license (the \`license\` field of \`package/package.json\`)
- Layout: the tarball's \`package/\` directory, extracted unchanged
PROV
  if [[ -n "$repo_block" ]]; then
    printf '%s\n' "$repo_block" >> "$dest/PROVENANCE.md"
  fi
  if [[ -n "$contents_block" ]]; then
    printf '\n%s\n' "$contents_block" >> "$dest/PROVENANCE.md"
  fi
  echo "   $(find "$dest/package" -type f | wc -l | tr -d ' ') files, license $license, sha256 $sha256"
}

# The file count and tree digest of the tree $1, without PROVENANCE.md.
tree_state() {
  if [[ -d "$1" ]]; then
    printf '%s %s\n' "$(corpus_file_count "$1")" "$(corpus_tree_digest "$1")"
  fi
}

# Verifies or fetches the examples package $1 into $examples_dir/$1, from the
# row of the docs/VERSIONS.md examples table whose Item cell names it: the
# version, then `files <count> digest <sha256>`. With $2 = 1 it takes the
# registry's bytes as they are and rewrites the PROVENANCE.md.
fetch_example() {
  local pkg="$1" stamp="$2" cell ver want_files want_digest dest meta shasum tarball_url tmp got sha256 license
  local files digest meta_url
  cell="$(corpus_pin_cell "$pkg")"
  ver="$(awk '{ print $1; exit }' <<< "$cell")"
  case "$ver" in
    "" | *[!A-Za-z0-9.-]*) die "pin for $pkg is not a plain version: '$ver'" ;;
  esac
  want_files="$(corpus_pin_field files "$cell")"
  want_digest="$(corpus_pin_field digest "$cell")"
  dest="$examples_dir/$pkg"
  if [[ "$stamp" -eq 0 ]]; then
    grep -qE '^[0-9]+$' <<< "$want_files" || die "the $pkg pin names no file count"
    grep -qE '^[0-9a-f]{64}$' <<< "$want_digest" || die "the $pkg pin names no tree digest"
    if [[ "$(tree_state "$dest")" = "$want_files $want_digest" ]]; then
      echo "   $pkg $ver: $want_files files at digest $want_digest, kept"
      return
    fi
  fi

  echo "== $pkg $ver"
  meta_url="$(meta_url_for "$pkg")"
  meta="$(curl --proto '=https' --tlsv1.2 -fsSL -A "$ua" "$meta_url")" \
    || die "cannot read registry metadata for $pkg"
  shasum="$(jq -r --arg v "$ver" '.versions[$v].dist.shasum // empty' <<< "$meta")"
  tarball_url="$(jq -r --arg v "$ver" '.versions[$v].dist.tarball // empty' <<< "$meta")"
  [[ -n "$shasum" ]] || die "the registry lists no version $ver of $pkg"
  [[ -n "$tarball_url" ]] || die "the registry lists no tarball for $pkg $ver"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  curl --proto '=https' --tlsv1.2 -fsSL -A "$ua" -o "$tmp/pkg.tgz" "$tarball_url" \
    || die "download of $pkg $ver failed"
  got="$(shasum -a 1 "$tmp/pkg.tgz" | cut -d' ' -f1)"
  [[ "$got" = "$shasum" ]] || die "checksum mismatch for $pkg $ver: registry $shasum, downloaded $got"
  sha256="$(shasum -a 256 "$tmp/pkg.tgz" | cut -d' ' -f1)"
  mkdir -p "$tmp/x"
  tar -xzf "$tmp/pkg.tgz" -C "$tmp/x"
  rm -f "$tmp/pkg.tgz"
  [[ -f "$tmp/x/package/package.json" ]] || die "$pkg $ver has no package/package.json"
  if find "$tmp/x" -type f \( -name 'sct2_*' -o -name 'der2_*' -o -name '*.rf2' \) | grep -q .; then
    die "$pkg $ver contains RF2-shaped files; SNOMED CT content is never vendored"
  fi
  license="$(jq -r '.license // "unstated"' "$tmp/x/package/package.json")"
  read -r files digest <<< "$(tree_state "$tmp/x")"
  if [[ "$stamp" -eq 0 ]]; then
    [[ "$files" = "$want_files" ]] || die "$pkg $ver stages $files files, the pin records $want_files"
    [[ "$digest" = "$want_digest" ]] || die "$pkg $ver stages the digest $digest, the pin records $want_digest"
    [[ -f "$dest/PROVENANCE.md" ]] || die "no $dest/PROVENANCE.md names the pin; run with --build-time --stamp"
    grep -qxF -- "- Version: $ver" "$dest/PROVENANCE.md" || die "$dest/PROVENANCE.md does not record version $ver"
    grep -qF "$digest" "$dest/PROVENANCE.md" || die "$dest/PROVENANCE.md does not name the digest $digest"
  fi
  mkdir -p "$dest"
  rm -rf "$dest/package"
  mv "$tmp/x/package" "$dest/package"
  git check-ignore -q "$dest/package" || die "$dest/package is not ignored by git; the tree must never be committed"
  if [[ "$stamp" -eq 1 ]]; then
    write_example_provenance "$pkg" "$ver" "$meta_url" "$tarball_url" "$shasum" "$sha256" "$license" "$files" "$digest"
    echo "   record in the $pkg row of docs/VERSIONS.md: $ver files $files digest $digest"
  fi
  echo "   $pkg $ver: $files files at digest $digest, license $license"
}

# Writes the PROVENANCE.md of the examples package $1 from what its fetch read.
write_example_provenance() {
  local pkg="$1" ver="$2" meta_url="$3" tarball_url="$4" shasum="$5" sha256="$6" license="$7" files="$8" digest="$9"
  local dir="$examples_dir/$pkg/package" resources types
  resources="$(find "$dir" -maxdepth 1 -type f -name '*.json' ! -name package.json ! -name '.index.json' | wc -l | tr -d '[:space:]')"
  types="$(find "$dir" -maxdepth 1 -type f -name '*.json' ! -name package.json ! -name '.index.json' -exec jq -r '.resourceType' {} + |
    LC_ALL=C sort -u | wc -l | tr -d '[:space:]')"
  cat > "$examples_dir/$pkg/PROVENANCE.md" << PROV
<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: $pkg

Fetched verbatim at build time by \`scripts/vendor/fhir-packages.sh
--build-time\` into \`package/\` beside this file, which \`.gitignore\` refuses
(.claude/rules/vendored-inputs.md). Never commit a file from there and never
edit one: change the pin in docs/VERSIONS.md, run the script with
\`--build-time --stamp\`, and record the count and digest it prints.

- Package: $pkg
- Version: $ver
- Source: the FHIR package registry, $meta_url
- Tarball: $tarball_url
- SHA-1 (registry shasum): $shasum
- SHA-256 (tarball): $sha256
- Upstream license: $license (the \`license\` field of \`package/package.json\`)
- Layout: the tarball's \`package/\` directory, extracted unchanged
- Files: $files
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`
- Contents: $resources resources of $types resource types, one JSON file each,
  beside \`package.json\` and \`.index.json\`
- Stamped: $(corpus_fetched)

## Why build time

The four examples packages unpack to about 600 MB, which would more than
double the 380 MB of FHIR packages that \`tools/fhir-codegen/vendor/\` commits,
for corpora only the conformance tests read. The licence would permit
vendoring; the size decides. CI restores the trees from a cache keyed on
\`scripts/vendor/fhir-packages.sh --cache-key\`, and the script verifies the
count and digest of a restored tree as it does of a download.

The examples are HL7's own published examples, never patient data.
PROV
}

case "${1:-}" in
  --cache-key)
    [[ "$#" -eq 1 ]] || die "usage: scripts/vendor/fhir-packages.sh --cache-key"
    keyed=""
    for pkg in "${example_packages[@]}"; do
      keyed="$keyed$pkg $(corpus_pin_cell "$pkg")
"
    done
    printf '%s' "$keyed" | shasum -a 256 | cut -c1-16
    exit 0
    ;;
  --build-time)
    stamp=0
    case "${2:-}" in
      "") ;;
      --stamp) stamp=1 ;;
      *) die "usage: scripts/vendor/fhir-packages.sh --build-time [--stamp]" ;;
    esac
    [[ "$#" -le 2 ]] || die "usage: scripts/vendor/fhir-packages.sh --build-time [--stamp]"
    command -v git >/dev/null 2>&1 || die "missing required tool: git"
    for pkg in "${example_packages[@]}"; do
      fetch_example "$pkg" "$stamp"
    done
    echo "fhir-packages: done"
    exit 0
    ;;
  *) ;;
esac

packages=("$@")
[[ "${#packages[@]}" -gt 0 ]] || packages=("${default_packages[@]}")
for pkg in "${packages[@]}"; do
  vendor_one "$pkg"
done
echo "fhir-packages: done"
