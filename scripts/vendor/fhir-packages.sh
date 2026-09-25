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
# Usage:
#   scripts/vendor/fhir-packages.sh                # the default set below
#   scripts/vendor/fhir-packages.sh hl7.terminology  # one or more packages
#
# Requires: curl, jq, tar, shasum.

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
  # The R6 ballots are published on packages2.fhir.org only; every other
  # package comes from packages.fhir.org. Both speak the npm registry shape.
  local meta_url
  case "$pkg" in
    hl7.fhir.r6.*) meta_url="$registry2/$pkg" ;;
    *) meta_url="$registry/$pkg" ;;
  esac
  meta="$(curl --proto '=https' --tlsv1.2 -fsSL -A "ferrobridge-vendor (scripts/vendor/fhir-packages.sh)" "$meta_url")" \
    || die "cannot read registry metadata for $pkg"
  shasum="$(printf '%s' "$meta" | jq -r --arg v "$ver" '.versions[$v].dist.shasum // empty')"
  tarball_url="$(printf '%s' "$meta" | jq -r --arg v "$ver" '.versions[$v].dist.tarball // empty')"
  [[ -n "$shasum" ]] || die "the registry lists no version $ver of $pkg"

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  [[ -n "$tarball_url" ]] || die "the registry lists no tarball for $pkg $ver"
  curl --proto '=https' --tlsv1.2 -fsSL -A "ferrobridge-vendor (scripts/vendor/fhir-packages.sh)" -o "$tmp/pkg.tgz" "$tarball_url" \
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

packages=("$@")
[[ "${#packages[@]}" -gt 0 ]] || packages=("${default_packages[@]}")
for pkg in "${packages[@]}"; do
  vendor_one "$pkg"
done
echo "fhir-packages: done"
