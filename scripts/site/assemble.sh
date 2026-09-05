#!/usr/bin/env bash
# SPDX-FileCopyrightText: Ruben Talstra
# SPDX-License-Identifier: BUSL-1.1
# assemble.sh: build the site the way GitHub Pages serves it, from the landing
# page and the book, into one directory.
#
#   scripts/site/assemble.sh OUT
#
# The landing page lands at the site root and the mdBook under /docs/, which is
# what book.toml's site-url declares. The landing page's roadmap block (between
# `<!-- roadmap:begin -->` and `<!-- roadmap:end -->`) is rendered here from the
# tracker's open milestones through the GitHub API when `gh` is authenticated.
# Without a token the block stays empty and the page keeps its link to the
# milestones page, so an assembly on a laptop never fails on the network.
#
# The brand assets (assets/brand) and llms.txt live outside the landing
# directory and are copied in here, because both are addressed from the site
# root.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

readonly OUT="${1:?usage: $0 OUT}"
readonly LANDING=website/landing
readonly REPO=rubentalstra/FerroBRIDGE

# The open milestones as list items, oldest due date first. Every field that
# reaches the page is HTML-escaped, because a milestone title is text somebody
# typed.
roadmap() {
  gh api "repos/$REPO/milestones?state=open&sort=due_on&direction=asc" --jq '
    def esc: gsub("&"; "&amp;") | gsub("<"; "&lt;") | gsub(">"; "&gt;");
    ["<ul class=\"roadmap\">"]
    + map("  <li><span class=\"tag\">" + (.title | esc) + "</span>"
          + "<a href=\"" + .html_url + "\" rel=\"noopener\">" + (.open_issues | tostring) + " open, " + (.closed_issues | tostring) + " closed</a>"
          + (if .due_on == null then "" else "<span class=\"due\">due " + .due_on[0:10] + "</span>" end)
          + "</li>")
    + ["</ul>"] | .[]'
}

mdbook build website/book

rm -rf "${OUT:?}"
mkdir -p "$OUT/docs"
cp -R "$LANDING"/. "$OUT/"
cp -R website/book/book/. "$OUT/docs/"

# The brand directory holds the favicons the landing page links and the social
# card its og:image names, so it has to reach the site root for those URLs to
# resolve (assets/brand/README.md).
mkdir -p "$OUT/assets"
cp -R assets/brand "$OUT/assets/"

# The llms.txt convention puts the file at the site root, which is the one URL
# that makes it useful.
cp llms.txt "$OUT/llms.txt"

block="$(mktemp)"
trap 'rm -f "$block"' EXIT

if roadmap > "$block" 2>/dev/null && [ -s "$block" ]; then
  awk -v blockfile="$block" '
    /<!-- roadmap:begin -->/ { print; while ((getline line < blockfile) > 0) print line; skip = 1; next }
    /<!-- roadmap:end -->/ { skip = 0 }
    !skip { print }
  ' "$LANDING/index.html" > "$OUT/index.html"
  echo "assemble: roadmap rendered from the open milestones."
else
  echo "assemble: no GitHub API access, so the roadmap block stays empty." >&2
fi

echo "assemble: site at $OUT"
