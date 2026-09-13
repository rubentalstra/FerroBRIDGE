#!/usr/bin/env bash
# SPDX-FileCopyrightText: Ruben Talstra
# SPDX-License-Identifier: BUSL-1.1
# The book theme favicons are copies of the brand favicon, not files of their
# own (assets/brand/README.md). mdBook reads a theme override from
# website/book/theme/favicon.svg and favicon.png, so the mark exists twice and
# nothing but this check keeps the two in step: change the brand mark, forget
# the copy, and the book serves the old favicon (#52).
#
#   scripts/checks/favicon-sync.sh
#
# Exit 0 when every copy is byte-identical to its source. Exit 1 naming the
# file that differs and the command that regenerates it.
set -euo pipefail
cd "$(dirname "$0")/../.."

# One "copy<TAB>source" record per line. The PNG source is the 32-pixel raster
# of the same SVG, which is what assets/brand/README.md's regeneration block
# writes to both paths.
readonly PAIRS="\
website/book/theme/favicon.svg	assets/brand/favicon.svg
website/book/theme/favicon.png	assets/brand/favicon-32.png"

fail=0
while IFS=$'\t' read -r copy source; do
  [ -n "$copy" ] || continue
  if [ ! -f "$source" ]; then
    echo "favicon-sync: $source is missing; the brand directory is the source of both copies." >&2
    fail=1
    continue
  fi
  if [ ! -f "$copy" ]; then
    echo "favicon-sync: $copy is missing; regenerate it from $source (assets/brand/README.md)." >&2
    fail=1
    continue
  fi
  if cmp -s "$copy" "$source"; then
    echo "favicon-sync: $copy matches $source"
    continue
  fi
  echo "::error file=$copy::$copy differs from $source. Re-run the regeneration block in assets/brand/README.md so the book theme serves the current mark." >&2
  fail=1
done <<< "$PAIRS"

exit "$fail"
