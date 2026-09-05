#!/usr/bin/env bash
# SPDX-FileCopyrightText: Ruben Talstra
# SPDX-License-Identifier: BUSL-1.1
# Fetch the browser assets the mermaid preprocessor needs into
# website/book/vendor/mermaid/, verbatim and checksum-verified.
#
# mdbook-mermaid turns a ```mermaid fence into a <pre class="mermaid"> block and
# leaves the rendering to the mermaid library in the browser, so the book needs
# both files on disk. `mdbook-mermaid install` writes the same two bytes; this
# script pins the commit instead of the local tool version, so the vendored tree
# is reproducible from the repository alone (.claude/rules/vendored-inputs.md).
#
# Usage:
#   scripts/vendor/mdbook-mermaid-assets.sh
#
# Re-run after changing a pin below, then commit the result and update
# website/book/vendor/mermaid/PROVENANCE.md.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/website/book/vendor/mermaid"

# mdbook-mermaid v0.17.1, the version .github/actions/docs-toolchain/action.yml
# installs and docs/VERSIONS.md pins.
readonly MDBOOK_MERMAID_COMMIT=99e2393a739f3c36fde30c59317d4daa355eefaf
# mermaid 11.6.0, the release vendored inside that commit's mermaid.min.js.
readonly MERMAID_COMMIT=7b2083926dbe3b6280f42376a0d4195b2c72fb8e

readonly RAW_MDBOOK_MERMAID="https://raw.githubusercontent.com/badboy/mdbook-mermaid/$MDBOOK_MERMAID_COMMIT"
readonly RAW_MERMAID="https://raw.githubusercontent.com/mermaid-js/mermaid/$MERMAID_COMMIT"

# One "URL  destination  sha256" record per line.
readonly FILES="\
$RAW_MDBOOK_MERMAID/src/bin/assets/mermaid.min.js mermaid.min.js eefea253bed9655e838eb874ff955c46872f982a8e26290c1dd2982ddc0a4703
$RAW_MDBOOK_MERMAID/src/bin/assets/mermaid-init.js mermaid-init.js 53b67c5380e8f4dac4cd080370cfbe69ee46aced71c597eeaa5f55a3a9877611
$RAW_MDBOOK_MERMAID/LICENSE LICENSE-mdbook-mermaid 1f256ecad192880510e84ad60474eab7589218784b9a50bc7ceee34c2b91f1d5
$RAW_MERMAID/LICENSE LICENSE-mermaid ec9fb67dcb25eccc416ed56e1aab819222c805a2a4bfe4cb19e7556bf2ffde80"

sha256_of() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

mkdir -p "$out"

while read -r url name want; do
  [ -n "$url" ] || continue
  tmp="$(mktemp)"
  curl --fail --silent --show-error --location --output "$tmp" "$url"
  got="$(sha256_of "$tmp")"
  if [ "$got" != "$want" ]; then
    rm -f "$tmp"
    printf 'vendor: %s changed upstream (want %s, got %s)\n' "$name" "$want" "$got" >&2
    exit 1
  fi
  mv "$tmp" "$out/$name"
  chmod 644 "$out/$name"
  printf 'vendor: %s ok\n' "$name"
done <<< "$FILES"

printf 'vendor: mermaid assets in %s\n' "${out#"$root/"}"
