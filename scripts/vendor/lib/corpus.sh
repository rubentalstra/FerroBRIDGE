# SPDX-FileCopyrightText: Ruben Talstra
# SPDX-License-Identifier: BUSL-1.1
# shellcheck shell=bash
# Shared helpers for the corpus vendor scripts (.claude/rules/vendored-inputs.md).
#
# Every corpus script sources this file, reads its pin from the corpus table in
# docs/VERSIONS.md, downloads that exact commit as a tarball from
# codeload.github.com, proves the archive carries the pinned commit (GitHub
# names the top directory `<repo>-<commit>`), and extracts the paths it names
# verbatim under docs/specs/<name>/ at their upstream layout. Re-running with an
# unchanged pin reproduces the tree byte for byte; only the fetch date in
# PROVENANCE.md moves.
#
# Requires: curl, tar, shasum, jq.
#
# No specification governs this file; it is FerroBRIDGE's own design.

corpus_ua="ferrobridge-vendor (scripts/vendor)"
corpus_matrix="docs/VERSIONS.md"
# The sourcing script names every message, so a failure says which corpus it
# belongs to without each script repeating its own name.
corpus_script="$(basename "${BASH_SOURCE[1]:-${BASH_SOURCE[0]}}" .sh)"

die() {
  printf '%s: %s\n' "$corpus_script" "$*" >&2
  exit 1
}

say() { printf '%s: %s\n' "$corpus_script" "$*"; }

corpus_require() {
  local tool
  for tool in "$@"; do
    command -v "$tool" > /dev/null 2>&1 || die "missing required tool: $tool"
  done
}

# The Pin cell of the docs/VERSIONS.md row whose Item cell is $1, backticks
# removed. This is the single source of truth for every pin (docs/VERSIONS.md).
corpus_pin_cell() {
  local cell
  cell="$(awk -F'|' -v item="$1" '
    NF >= 3 {
      k = $2; v = $3
      gsub(/`/, "", k); gsub(/`/, "", v)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", k)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
      if (k == item) { print v; exit }
    }
  ' "$corpus_matrix")"
  [ -n "$cell" ] || die "$corpus_matrix has no '$1' row"
  printf '%s\n' "$cell"
}

# The owner/repo a pin cell names: its first token.
corpus_pin_repo() {
  local repo
  repo="$(awk '{ print $1; exit }' <<< "$1")"
  case "$repo" in
    */*) printf '%s\n' "$repo" ;;
    *) die "the pin '$1' does not open with an owner/repo" ;;
  esac
}

# The commit a pin cell names: its first 40-hex token.
corpus_pin_commit() {
  local commit
  commit="$(grep -oE '[0-9a-f]{40}' <<< "$1" | head -n1)"
  [ -n "$commit" ] || die "the pin '$1' names no commit"
  printf '%s\n' "$commit"
}

# The tag a pin cell names: the token after the word `tag`, without trailing
# punctuation.
corpus_pin_tag() {
  local tag
  tag="$(awk '{
    for (i = 1; i < NF; i++) {
      if ($i == "tag") { t = $(i + 1); gsub(/[,.;:]+$/, "", t); print t; exit }
    }
  }' <<< "$1")"
  [ -n "$tag" ] || die "the pin '$1' names no tag"
  printf '%s\n' "$tag"
}

corpus_api() {
  curl --proto '=https' --tlsv1.2 -fsSL -A "$corpus_ua" \
    -H 'Accept: application/vnd.github+json' "$1"
}

# The commit a tag resolves to. A tag is mutable, so the resolved commit is what
# the provenance records; an annotated tag needs the extra dereference hop.
corpus_resolve_tag() {
  local repo="$1" tag="$2" ref kind sha url
  ref="$(corpus_api "https://api.github.com/repos/$repo/git/ref/tags/$tag")" \
    || die "cannot resolve tag $tag of $repo"
  kind="$(jq -r '.object.type // empty' <<< "$ref")"
  sha="$(jq -r '.object.sha // empty' <<< "$ref")"
  if [ "$kind" = "tag" ]; then
    url="$(jq -r '.object.url // empty' <<< "$ref")"
    sha="$(corpus_api "$url" | jq -r '.object.sha // empty')"
  fi
  grep -qE '^[0-9a-f]{40}$' <<< "$sha" || die "tag $tag of $repo resolves to no commit"
  printf '%s\n' "$sha"
}

# Downloads $1 at commit $2 into the scratch directory $3 and prints the
# extracted root. The archive's top directory carries the commit, which is the
# proof that the bytes on disk are the pinned ones.
corpus_fetch() {
  local repo="$1" commit="$2" into="$3" root
  curl --proto '=https' --tlsv1.2 -fsSL -A "$corpus_ua" \
    -o "$into/corpus.tar.gz" "https://codeload.github.com/$repo/tar.gz/$commit" \
    || die "download of $repo at $commit failed"
  mkdir -p "$into/tree"
  tar -xzf "$into/corpus.tar.gz" -C "$into/tree"
  root="$into/tree/${repo##*/}-$commit"
  [ -d "$root" ] || die "the archive of $repo does not carry commit $commit"
  printf '%s\n' "$root"
}

# Copies each named path from the archive root $1 to $2, keeping the upstream
# layout so a vendored file can be compared with its source by path.
corpus_take() {
  local src="$1" dest="$2" path
  shift 2
  for path in "$@"; do
    [ -e "$src/$path" ] || die "the archive has no $path"
    mkdir -p "$dest/$(dirname "$path")"
    cp -R "$src/$path" "$dest/$path"
  done
}

corpus_sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }

# The git blob id of a file: the object name git records for those bytes
# (<https://git-scm.com/book/en/v2/Git-Internals-Git-Objects>).
corpus_blob_id() {
  local size
  size="$(wc -c < "$1" | tr -d '[:space:]')"
  {
    printf 'blob %s\0' "$size"
    cat "$1"
  } | shasum -a 1 | cut -d' ' -f1
}

# One digest over a whole vendored tree: sha256 of the sorted per-file
# `sha256  path` listing, with the provenance stamps themselves left out so the
# digest covers only upstream bytes.
corpus_tree_digest() {
  (
    cd "$1" || exit 1
    find . -type f ! -name PROVENANCE.md | LC_ALL=C sort |
      while IFS= read -r f; do shasum -a 256 "$f"; done |
      shasum -a 256 | cut -d' ' -f1
  )
}

corpus_file_count() { find "$1" -type f ! -name PROVENANCE.md | wc -l | tr -d '[:space:]'; }

corpus_fetched() { date -u +%Y-%m-%d; }
