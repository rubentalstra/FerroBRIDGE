#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# Every CI tool pin no ecosystem watches, compared with its newest upstream
# release (#35). Dependabot's `github-actions` ecosystem reads `uses:`
# references; it does not read an image named in a `run:` block, a version
# handed to an installer as an input, or a commit a vendor script fetches from.
# scripts/checks/versions.sh catches drift between two files we control and
# cannot tell you that upstream published a newer release, which is what this
# reads.
#
#   scripts/checks/pin-freshness.sh
#
# Reads each pin from docs/VERSIONS.md and each newest release tag from the
# upstream project's GitHub releases, which is the tag the container image and
# the installer both carry. A corpus pinned by commit on a repository with no
# releases is read against the newest commit of the branch it follows. Needs an
# authenticated `gh`.
#
# Exit 0 when every pin is current, 1 when at least one is behind (each such
# line starts with STALE), 2 when a release could not be read, so a network
# failure never reads as a fresh pin.
set -euo pipefail
cd "$(dirname "$0")/../.."

readonly MATRIX=docs/VERSIONS.md

# One "matrix label<TAB>upstream repository" record per line. The label is the
# first cell of the row in docs/VERSIONS.md, backticks and all.
readonly WATCHED="\
zizmor	zizmorcore/zizmor
actionlint	rhysd/actionlint
shellcheck	koalaman/shellcheck
hadolint	hadolint/hadolint
mdBook	rust-lang/mdBook
mdbook-toc	badboy/mdbook-toc
mdbook-mermaid	badboy/mdbook-mermaid"

# The second cell of the matrix row whose first cell is $1, with the backticks
# stripped and only the first token kept, the same shape versions.sh reads.
matrix_pin() {
  awk -F'|' -v want="$1" '
    NF >= 3 {
      label = $2; value = $3
      gsub(/`/, "", label); gsub(/^[ \t]+|[ \t]+$/, "", label)
      gsub(/^[ \t]+|[ \t]+$/, "", value)
      if (label == want) { split(value, f, " "); print f[1]; exit }
    }' "$MATRIX"
}

stale=0
unreadable=0
while IFS=$'\t' read -r label repo; do
  [ -n "$label" ] || continue

  pinned="$(matrix_pin "$label")"
  if [ -z "$pinned" ]; then
    printf 'UNREADABLE %s: no pin row in %s\n' "$label" "$MATRIX"
    unreadable=1
    continue
  fi

  if ! tag="$(gh api "repos/$repo/releases/latest" --jq '.tag_name' 2>&1)"; then
    printf 'UNREADABLE %s: could not read the newest release of %s (%s)\n' "$label" "$repo" "$tag"
    unreadable=1
    continue
  fi
  latest="${tag#v}"

  if [ "$pinned" = "$latest" ]; then
    printf 'current    %s %s (%s)\n' "$label" "$pinned" "$repo"
  else
    printf 'STALE      %s: pinned %s, newest upstream release %s (https://github.com/%s/releases/tag/%s)\n' \
      "$label" "$pinned" "$latest" "$repo" "$tag"
    stale=1
  fi
done <<< "$WATCHED"

# One "matrix label<TAB>upstream repository<TAB>branch" record per line: a
# corpus pinned by commit on a repository that publishes no releases, read
# against the newest commit of the branch the pin follows.
# TODO(#268): the other commit-pinned corpora and the FHIR packages.
readonly WATCHED_COMMITS="\
HL7 v2 samples: Microsoft FHIR-Converter	microsoft/FHIR-Converter	main
HL7 v2 samples: CDC ReportStream data tests	CDCgov/prime-reportstream	main
HL7 v2 samples: HL7 v2-to-FHIR benchmark messages	HL7/v2-to-fhir	master
HL7 v2 samples: NIST LRI (build time, never committed)	usnistgov/hit-mu-tools-resource-bundles	lri-r2
HL7 v2 samples: NIST LOI (build time, never committed)	usnistgov/hit-mu-tools-resource-bundles	loi-r1
HL7 v2 samples: NIST syndromic surveillance (build time, never committed)	usnistgov/hit-mu-tools-resource-bundles	ss-r2
HL7 v2 samples: AIRA MQE (build time, never committed)	immregistries/mqe	master"

# The first 40-hex commit in the second cell of the matrix row whose first cell
# is $1.
matrix_commit() {
  awk -F'|' -v want="$1" '
    NF >= 3 {
      label = $2; value = $3
      gsub(/`/, "", label); gsub(/^[ \t]+|[ \t]+$/, "", label)
      if (label == want && match(value, /[0-9a-f]{40}/)) { print substr(value, RSTART, RLENGTH); exit }
    }' "$MATRIX"
}

while IFS=$'\t' read -r label repo branch; do
  [ -n "$label" ] || continue

  pinned="$(matrix_commit "$label")"
  if [ -z "$pinned" ]; then
    printf 'UNREADABLE %s: no commit pin in %s\n' "$label" "$MATRIX"
    unreadable=1
    continue
  fi

  if ! head="$(gh api "repos/$repo/commits/$branch" --jq '.sha' 2>&1)"; then
    printf 'UNREADABLE %s: could not read the head of %s %s (%s)\n' "$label" "$repo" "$branch" "$head"
    unreadable=1
    continue
  fi

  if [ "$pinned" = "$head" ]; then
    printf 'current    %s %s (%s %s)\n' "$label" "$pinned" "$repo" "$branch"
  else
    printf 'STALE      %s: pinned %s, newest commit on %s %s (https://github.com/%s/commit/%s)\n' \
      "$label" "$pinned" "$branch" "$head" "$repo" "$head"
    stale=1
  fi
done <<< "$WATCHED_COMMITS"

[ "$unreadable" -eq 0 ] || exit 2
exit "$stale"
