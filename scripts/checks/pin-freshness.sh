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
# the installer both carry. Needs an authenticated `gh`.
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

[ "$unreadable" -eq 0 ] || exit 2
exit "$stale"
