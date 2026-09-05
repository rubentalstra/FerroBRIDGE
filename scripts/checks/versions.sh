#!/usr/bin/env bash
# SPDX-FileCopyrightText: Ruben Talstra
# SPDX-License-Identifier: BUSL-1.1
# Version-drift guard (docs/VERSIONS.md is the single source of truth).
#
# Every file that repeats a pin must agree with the matrix. The repository is
# in its design phase, so a check whose subject file is absent SKIPS LOUDLY
# with a printed reason, and gains teeth the moment the file appears.
#
#   1. specification pins  the five rows of the docs/architecture.md pin table
#                          (FHIRconnect, FHIR, OMOCL, OMOP CDM, openEHR
#                          ITS-REST) against docs/VERSIONS.md.
#   2. FHIR model crates   fhir-types and fhir-terminology across
#                          docs/architecture.md, docs/VERSIONS.md, and the root
#                          Cargo.toml [workspace.dependencies] requirement.
#   3. toolchain           rust-toolchain.toml channel, plus the root
#                          Cargo.toml edition, rust-version and resolver.
#   4. product version     CITATION.cff version against the docs/VERSIONS.md
#                          product-version row, and against the root Cargo.toml
#                          [workspace.package] version once that exists.
#   5. CI tool pins        the zizmor, actionlint, shellcheck and hadolint
#                          versions .github/workflows/ci.yml installs, against
#                          docs/VERSIONS.md.
#   6. docs toolchain      the mdBook, mdbook-toc and mdbook-mermaid defaults of
#                          .github/actions/docs-toolchain/action.yml against
#                          docs/VERSIONS.md.
#   7. licence             LICENSE is the Business Source License 1.1 and no
#                          first-party file claims MIT or Apache-2.0 as its own.
#
# Usage:
#   scripts/checks/versions.sh
#
# Exit 0 = every present check agrees (skips are fine). Exit 1 = a real drift.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

fail=0
note() { printf '  %s\n' "$*"; }
bad() {
  printf '  DRIFT: %s\n' "$*" >&2
  fail=1
}

# The first whitespace-separated token of the second cell of the markdown table
# row whose first cell is ITEM, with surrounding spaces and backticks removed.
pin_of() {
  awk -F'|' -v item="$1" '
    NF >= 3 {
      k = $2; v = $3
      gsub(/`/, "", k); gsub(/`/, "", v)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", k)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
      if (k == item) { split(v, w, /[[:space:]]/); print w[1]; exit }
    }
  ' "$2"
}

# The value of KEY inside TOML table TABLE, unquoted.
toml_val() {
  awk -v table="$1" -v key="$2" '
    /^[[:space:]]*\[/ { h = $0; gsub(/[[:space:]]/, "", h); f = (h == table); next }
    f && $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
      if (match($0, /"[^"]*"/)) { print substr($0, RSTART + 1, RLENGTH - 2); exit }
      sub(/^[^=]*=[[:space:]]*/, "")
      gsub(/[[:space:]]/, "")
      print; exit
    }
  ' "$3"
}

# The version requirement of dependency NAME in the root Cargo.toml, in either
# the `name = "x.y.z"` or the `name = { version = "x.y.z" }` form.
manifest_req() {
  awk -v name="$1" '
    $0 ~ "^[[:space:]]*" name "[[:space:]]*=" {
      if (match($0, /version[[:space:]]*=[[:space:]]*"[^"]+"/)) {
        s = substr($0, RSTART, RLENGTH)
      } else if (match($0, /=[[:space:]]*"[^"]+"/)) {
        s = substr($0, RSTART, RLENGTH)
      } else { next }
      match(s, /"[^"]+"/)
      print substr(s, RSTART + 1, RLENGTH - 2); exit
    }
  ' Cargo.toml
}

# The `default:` of composite-action input KEY, unquoted. An input key sits at
# two spaces of indentation and its own keys at four, which is what the exact
# prefix comparisons below rely on.
action_default() {
  awk -v key="  $1:" '
    $0 == key { inside = 1; next }
    inside && index($0, "    default:") == 1 {
      sub(/^[[:space:]]*default:[[:space:]]*/, "")
      gsub(/"/, "")
      gsub(/[[:space:]]/, "")
      print
      exit
    }
    inside && $0 ~ /^[^[:space:]]/ { exit }
  ' "$2"
}

echo "== specification pins (docs/architecture.md <-> docs/VERSIONS.md)"
if [ -f docs/architecture.md ] && [ -f docs/VERSIONS.md ]; then
  agreed=0
  for item in "FHIRconnect" "FHIR" "OMOCL" "OMOP CDM" "openEHR ITS-REST"; do
    arch="$(pin_of "$item" docs/architecture.md)"
    matrix="$(pin_of "$item" docs/VERSIONS.md)"
    if [ -z "$arch" ]; then
      bad "docs/architecture.md has no '$item' pin row"
    elif [ -z "$matrix" ]; then
      bad "docs/VERSIONS.md has no '$item' pin row"
    elif [ "$arch" != "$matrix" ]; then
      bad "$item: docs/architecture.md says $arch, docs/VERSIONS.md pins $matrix"
    else
      agreed=$((agreed + 1))
    fi
  done
  [ "$agreed" -eq 5 ] && note "OK: all five specification pins agree"
else
  note "no docs/architecture.md or docs/VERSIONS.md yet, skipped"
fi

echo "== FHIR model crate pins (docs/architecture.md <-> docs/VERSIONS.md <-> Cargo.toml)"
if [ -f docs/architecture.md ] && [ -f docs/VERSIONS.md ]; then
  for crate in fhir-types fhir-terminology; do
    arch="$(pin_of "$crate" docs/architecture.md)"
    matrix="$(pin_of "$crate" docs/VERSIONS.md)"
    if [ -z "$arch" ]; then
      bad "docs/architecture.md has no $crate row"
      continue
    elif [ -z "$matrix" ]; then
      bad "docs/VERSIONS.md has no $crate row"
      continue
    elif [ "$arch" != "$matrix" ]; then
      bad "$crate: docs/architecture.md says $arch, docs/VERSIONS.md pins $matrix"
      continue
    fi
    note "OK: $crate $matrix (architecture and matrix agree)"
    if [ -f Cargo.toml ]; then
      req="$(manifest_req "$crate")"
      if [ -z "$req" ]; then
        note "root Cargo.toml has no $crate requirement yet, skipped"
      elif [ "$req" != "$matrix" ]; then
        bad "$crate: root Cargo.toml requires $req, docs/VERSIONS.md pins $matrix"
      else
        note "OK: root Cargo.toml requires $crate $req"
      fi
    else
      note "no root Cargo.toml yet, skipped the $crate requirement"
    fi
  done
else
  note "no docs/architecture.md or docs/VERSIONS.md yet, skipped"
fi

echo "== toolchain (rust-toolchain.toml and Cargo.toml <-> docs/VERSIONS.md)"
if [ -f rust-toolchain.toml ]; then
  chan="$(toml_val "[toolchain]" channel rust-toolchain.toml)"
  pin="$(pin_of "Rust toolchain" docs/VERSIONS.md)"
  if [ -z "$chan" ]; then
    bad "rust-toolchain.toml has no [toolchain] channel"
  elif [ -z "$pin" ]; then
    bad "docs/VERSIONS.md has no 'Rust toolchain' row"
  elif [ "$chan" != "$pin" ]; then
    bad "toolchain: rust-toolchain.toml channel is $chan, docs/VERSIONS.md pins $pin"
  else
    note "OK: the toolchain is $chan"
  fi
else
  note "no rust-toolchain.toml yet, skipped"
fi

if [ -f Cargo.toml ]; then
  edition="$(toml_val "[workspace.package]" edition Cargo.toml)"
  msrv="$(toml_val "[workspace.package]" rust-version Cargo.toml)"
  resolver="$(toml_val "[workspace]" resolver Cargo.toml)"
  check_row() {
    local label="$1" found="$2" row="$3"
    local want
    want="$(pin_of "$row" docs/VERSIONS.md)"
    if [ -z "$found" ]; then
      note "root Cargo.toml has no $label yet, skipped"
    elif [ -z "$want" ]; then
      bad "docs/VERSIONS.md has no '$row' row"
    elif [ "$found" != "$want" ]; then
      bad "$label: root Cargo.toml says $found, docs/VERSIONS.md pins $want"
    else
      note "OK: $label is $found"
    fi
  }
  check_row edition "$edition" "Edition"
  check_row rust-version "$msrv" "MSRV"
  check_row resolver "$resolver" "Cargo resolver"
else
  note "no root Cargo.toml yet, skipped the edition, MSRV and resolver rows"
fi

echo "== product version (CITATION.cff <-> docs/VERSIONS.md <-> Cargo.toml)"
if [ -f CITATION.cff ]; then
  cff="$(sed -nE 's/^version:[[:space:]]*//p' CITATION.cff | head -n1 | tr -d '"'\''[:space:]')"
  matrix_ver="$(pin_of "Product version" docs/VERSIONS.md)"
  if [ -z "$cff" ]; then
    bad "CITATION.cff has no version"
  elif [ -z "$matrix_ver" ]; then
    bad "docs/VERSIONS.md has no 'Product version' row"
  elif [ "$cff" != "$matrix_ver" ]; then
    bad "product version: CITATION.cff says $cff, docs/VERSIONS.md pins $matrix_ver"
  else
    note "OK: CITATION.cff and docs/VERSIONS.md both name $cff"
  fi
  if [ -f Cargo.toml ]; then
    cargo_ver="$(toml_val "[workspace.package]" version Cargo.toml)"
    if [ -z "$cargo_ver" ]; then
      bad "root Cargo.toml has no [workspace.package] version"
    elif [ -n "$cff" ] && [ "$cff" != "$cargo_ver" ]; then
      bad "product version: CITATION.cff says $cff, root Cargo.toml says $cargo_ver"
    else
      note "OK: root Cargo.toml names $cargo_ver"
    fi
  else
    note "no root Cargo.toml yet, skipped its version"
  fi
else
  note "no CITATION.cff yet, skipped"
fi

echo "== CI tool pins (.github/workflows/ci.yml <-> docs/VERSIONS.md)"
if [ -f .github/workflows/ci.yml ]; then
  # The version each analyzer is pinned to in the workflow: an installer
  # `tool: name@version` line, or the tag of a digest-pinned image.
  ci_tool_pin() {
    case "$1" in
    zizmor | shellcheck)
      sed -nE "s|^[[:space:]]*tool:[[:space:]]*$1@([^[:space:]]+).*|\1|p" \
        .github/workflows/ci.yml | head -n1
      ;;
    actionlint)
      sed -nE 's|^[[:space:]]*rhysd/actionlint:([^@[:space:]]+)@sha256:.*|\1|p' \
        .github/workflows/ci.yml | head -n1
      ;;
    hadolint)
      sed -nE 's|^[[:space:]]*hadolint/hadolint:v([^@[:space:]]+)@sha256:.*|\1|p' \
        .github/workflows/ci.yml | head -n1
      ;;
    esac
  }
  for tool in zizmor actionlint shellcheck hadolint; do
    want="$(pin_of "$tool" docs/VERSIONS.md)"
    found="$(ci_tool_pin "$tool")"
    if [ -z "$want" ]; then
      bad "docs/VERSIONS.md has no '$tool' row"
    elif [ -z "$found" ]; then
      bad ".github/workflows/ci.yml pins no $tool version"
    elif [ "$found" != "$want" ]; then
      bad "$tool: ci.yml pins $found, docs/VERSIONS.md pins $want"
    else
      note "OK: $tool $found"
    fi
  done
else
  note "no .github/workflows/ci.yml yet, skipped"
fi

echo "== docs toolchain (.github/actions/docs-toolchain <-> docs/VERSIONS.md)"
action=.github/actions/docs-toolchain/action.yml
if [ -f "$action" ]; then
  agreed=0
  for tool in mdbook mdbook-toc mdbook-mermaid; do
    if [ "$tool" = mdbook ]; then row=mdBook; else row="$tool"; fi
    found="$(action_default "$tool-version" "$action")"
    want="$(pin_of "$row" docs/VERSIONS.md)"
    if [ -z "$found" ]; then
      bad "$action has no $tool-version default"
    elif [ -z "$want" ]; then
      bad "docs/VERSIONS.md has no '$row' row"
    elif [ "$found" != "$want" ]; then
      bad "$tool: $action installs $found, docs/VERSIONS.md pins $want"
    else
      agreed=$((agreed + 1))
    fi
  done
  [ "$agreed" -eq 3 ] && note "OK: the three docs-toolchain pins agree"
else
  note "no $action yet, skipped"
fi

echo "== licence (LICENSE <-> SPDX headers, manifests, badges, labels)"
if [ -f LICENSE ]; then
  stale=0
  if ! grep -q 'Business Source License 1.1' LICENSE; then
    bad "LICENSE is not the Business Source License 1.1"
    stale=1
  fi
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    bad "stale licence claim at $hit"
    stale=1
  done < <(git grep -n -E 'SPDX-License-Identifier: (MIT|Apache-2\.0)|License-MIT|License-Apache|^license = "(MIT|Apache-2\.0)"|^license: (MIT|Apache-2\.0)|image\.licenses="?(MIT|Apache)' \
    -- ':!LICENSE' ':!CHANGELOG.md' ':!scripts/checks/versions.sh' ':(glob,exclude)**/vendor/**' || true)
  [ "$stale" -eq 0 ] && note "OK: every first-party file names BUSL-1.1"
else
  note "no LICENSE yet, skipped"
fi

echo
if [ "$fail" -ne 0 ]; then
  echo "versions: DRIFT detected" >&2
  exit 1
fi
echo "versions: OK (every present check agrees)"
