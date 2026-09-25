#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
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
#   2. model crates        fhir-types and the openehr-* crates across
#                          docs/architecture.md, docs/VERSIONS.md, and the root
#                          Cargo.toml [workspace.dependencies] requirement,
#                          and the testcontainers, tokio-postgres and sqlx
#                          rows of docs/VERSIONS.md against that requirement.
#   3. toolchain          rust-toolchain.toml channel, plus the root
#                          Cargo.toml edition, rust-version and resolver.
#   4. product version     CITATION.cff version against the docs/VERSIONS.md
#                          product-version row, and against the root Cargo.toml
#                          [workspace.package] version once that exists.
#   5. CI tool pins        the zizmor, actionlint, shellcheck, hadolint and
#                          sqlx-cli versions .github/workflows/ci.yml installs
#                          (sqlx-cli also against the sqlx crate row), its
#                          sqlx-offline service image against the PostgreSQL
#                          row, and the
#                          cargo-auditable, cargo-cyclonedx and syft versions
#                          the two release workflows install, against
#                          docs/VERSIONS.md.
#   6. docs toolchain      the mdBook, mdbook-toc and mdbook-mermaid defaults of
#                          .github/actions/docs-toolchain/action.yml against
#                          docs/VERSIONS.md.
#   7. container images    the PinnedImage constants of the testkit container
#                          harness against the docs/VERSIONS.md image rows.
#   8. image and compose   the FROM of docker/Dockerfile against the base-image
#                          row, the compose.yaml PostgreSQL image against the
#                          PostgreSQL row, and the compose.yaml bridge tag
#                          against the product version and the workspace
#                          version.
#   9. vendored corpora    every docs/specs/*/PROVENANCE.md, the vendored
#                          fixtures and the build-time HL7 v2 definitions name
#                          the commit or tag the docs/VERSIONS.md corpus row
#                          pins for them, and every sha256 or tree digest the
#                          row records.
#  10. FHIR packages       every row of the docs/VERSIONS.md FHIR-packages
#                          table against the Version line of its
#                          tools/fhir-codegen/vendor/<package>/PROVENANCE.md,
#                          and a package vendored with its source repository
#                          records that repository's licence.
#  11. licence            LICENSE is the Business Source License 1.1 and no
#                          first-party file claims MIT or Apache-2.0 as its
#                          own, crates/fhir-types and crates/hl7v2-types
#                          excepted (Apache-2.0).
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

echo "== model crate pins (docs/architecture.md <-> docs/VERSIONS.md <-> Cargo.toml)"
if [ -f docs/architecture.md ] && [ -f docs/VERSIONS.md ]; then
  for crate in fhir-types openehr-base openehr-rm openehr-its openehr-query; do
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

echo "== dependency crate pins (docs/VERSIONS.md <-> Cargo.toml)"
if [ -f docs/VERSIONS.md ] && [ -f Cargo.toml ]; then
  agreed=0
  for crate in testcontainers tokio-postgres sqlx; do
    matrix="$(pin_of "$crate" docs/VERSIONS.md)"
    req="$(manifest_req "$crate")"
    if [ -z "$matrix" ]; then
      bad "docs/VERSIONS.md has no $crate row"
    elif [ -z "$req" ]; then
      bad "root Cargo.toml has no $crate requirement"
    elif [ "$req" != "$matrix" ]; then
      bad "$crate: root Cargo.toml requires $req, docs/VERSIONS.md pins $matrix"
    else
      agreed=$((agreed + 1))
    fi
  done
  [ "$agreed" -eq 3 ] && note "OK: all three dependency crate pins agree"
else
  note "no docs/VERSIONS.md or root Cargo.toml yet, skipped"
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
    zizmor | shellcheck | sqlx-cli)
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
  for tool in zizmor actionlint shellcheck hadolint sqlx-cli; do
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

  # The CLI that writes the query metadata and the crate that reads it are
  # released together, so they carry one version.
  cli="$(pin_of sqlx-cli docs/VERSIONS.md)"
  crate="$(pin_of sqlx docs/VERSIONS.md)"
  if [ -n "$cli" ] && [ "$cli" != "$crate" ]; then
    bad "sqlx-cli is pinned at $cli, the sqlx crate at $crate"
  fi

  ci_pg="$(sed -nE 's|^[[:space:]]*image:[[:space:]]*(postgres:[^[:space:]]+)[[:space:]]*$|\1|p' .github/workflows/ci.yml | head -n1)"
  want_pg="$(pin_of "PostgreSQL image" docs/VERSIONS.md)"
  if [ -z "$ci_pg" ]; then
    bad ".github/workflows/ci.yml has no digest-pinned postgres service image"
  elif [ "$ci_pg" != "$want_pg" ]; then
    bad "sqlx-offline service: ci.yml runs $ci_pg, docs/VERSIONS.md pins $want_pg"
  else
    note "OK: the sqlx-offline service runs $ci_pg"
  fi
else
  note "no .github/workflows/ci.yml yet, skipped"
fi

echo "== release and fuzz tool pins (.github/workflows/{release-*,fuzz}.yml <-> docs/VERSIONS.md)"
# Every version of TOOL that the release workflows install, deduplicated, so a
# tool named in both files has to carry the same pin in both.
release_tool_pins() {
  local tool="$1" wf
  for wf in .github/workflows/release-build.yml .github/workflows/release-image.yml .github/workflows/fuzz.yml; do
    [ -f "$wf" ] || continue
    sed -nE "s|^[[:space:]]*tool:[[:space:]]*${tool}@([^[:space:]]+).*|\1|p" "$wf"
  done | sort -u
}

if [ -f .github/workflows/release-build.yml ] || [ -f .github/workflows/release-image.yml ]; then
  agreed=0
  for tool in cargo-auditable cargo-cyclonedx syft cargo-fuzz; do
    want="$(pin_of "$tool" docs/VERSIONS.md)"
    found="$(release_tool_pins "$tool")"
    if [ -z "$want" ]; then
      bad "docs/VERSIONS.md has no '$tool' row"
    elif [ -z "$found" ]; then
      bad "the release workflows pin no $tool version"
    elif [ "$(printf '%s\n' "$found" | wc -l | tr -d '[:space:]')" != "1" ]; then
      bad "$tool: the release workflows disagree ($(printf '%s' "$found" | tr '\n' ' '))"
    elif [ "$found" != "$want" ]; then
      bad "$tool: the release workflows pin $found, docs/VERSIONS.md pins $want"
    else
      agreed=$((agreed + 1))
      note "OK: $tool $found"
    fi
  done
  [ "$agreed" -eq 3 ] && note "OK: all three release tool pins agree"
else
  note "no release-build.yml or release-image.yml yet, skipped"
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

echo "== container images (tools/ferrobridge-testkit <-> docs/VERSIONS.md)"
harness=tools/ferrobridge-testkit/src/containers.rs
if [ -f "$harness" ] && [ -f docs/VERSIONS.md ]; then
  # The repository, tag and digest of the PinnedImage literal named CONST,
  # composed into the one reference the matrix row carries.
  image_pin_of() {
    awk -v name="$1" '
      $0 ~ "^pub const " name ": PinnedImage = PinnedImage \\{" { inside = 1; next }
      inside {
        if ($0 ~ /^\};/) { exit }
        if (match($0, /repository: "[^"]+"/)) { repo = substr($0, RSTART + 13, RLENGTH - 14) }
        if (match($0, /tag: "[^"]+"/)) { tag = substr($0, RSTART + 6, RLENGTH - 7) }
        if (match($0, /digest: "[^"]+"/)) { digest = substr($0, RSTART + 9, RLENGTH - 10) }
      }
      END { if (repo != "" && tag != "" && digest != "") print repo ":" tag "@" digest }
    ' "$2"
  }

  agreed=0
  expected=0
  for image in \
    "PostgreSQL image|POSTGRES" \
    "FerroEHR CDR image|CDR" \
    "FerroEHR CDR database image|CDR_POSTGRES" \
    "FerroTERM terminology server image|TERMINOLOGY"; do
    item="${image%%|*}"
    constant="${image##*|}"
    expected=$((expected + 1))
    want="$(pin_of "$item" docs/VERSIONS.md)"
    found="$(image_pin_of "$constant" "$harness")"
    if [ -z "$want" ]; then
      bad "docs/VERSIONS.md has no '$item' row"
    elif [ -z "$found" ]; then
      bad "$harness has no $constant PinnedImage with a repository, tag and digest"
    elif [ "$found" != "$want" ]; then
      bad "$item: $harness pins $found, docs/VERSIONS.md pins $want"
    else
      agreed=$((agreed + 1))
    fi
  done
  [ "$agreed" -eq "$expected" ] && note "OK: all $expected container image pins agree"
else
  note "no $harness yet, skipped"
fi

echo "== container recipe and quickstart (docker/Dockerfile, compose.yaml <-> docs/VERSIONS.md)"
if [ -f docker/Dockerfile ] && [ -f docs/VERSIONS.md ]; then
  base="$(sed -nE 's|^FROM[[:space:]]+([^[:space:]]+).*|\1|p' docker/Dockerfile | head -n1)"
  want_base="$(pin_of "Container base image" docs/VERSIONS.md)"
  if [ -z "$base" ]; then
    bad "docker/Dockerfile has no FROM"
  elif [ -z "$want_base" ]; then
    bad "docs/VERSIONS.md has no 'Container base image' row"
  elif [ "$base" != "$want_base" ]; then
    bad "base image: docker/Dockerfile builds on $base, docs/VERSIONS.md pins $want_base"
  else
    note "OK: the base image is $base"
  fi
  # The digest belongs to the FROM alone; the base.name label names the tag it
  # came from, so a bump that moves one and not the other is caught here.
  label_base="$(sed -nE 's|.*org\.opencontainers\.image\.base\.name="([^"]+)".*|\1|p' docker/Dockerfile | head -n1)"
  if [ -n "$label_base" ] && [ "${base%%@*}" != "$label_base" ]; then
    bad "base image: the base.name label says $label_base, the FROM is ${base%%@*}"
  fi
else
  note "no docker/Dockerfile yet, skipped"
fi

if [ -f compose.yaml ] && [ -f docs/VERSIONS.md ]; then
  # Every ferrobridge image reference in the quickstart carries the same tag
  # default, so the set is collapsed and a second value is drift by itself.
  tags="$(sed -nE 's|^[[:space:]]*image:[[:space:]]*ghcr\.io/rubentalstra/ferrobridge:\$\{[A-Za-z_][A-Za-z0-9_]*:-([^}]+)\}[[:space:]]*$|\1|p' compose.yaml | sort -u)"
  count="$(printf '%s\n' "$tags" | grep -c . || true)"
  matrix_product="$(pin_of "Product version" docs/VERSIONS.md)"
  if [ "$count" -eq 0 ]; then
    bad "compose.yaml has no ghcr.io/rubentalstra/ferrobridge image tag default"
  elif [ "$count" -ne 1 ]; then
    bad "compose.yaml names more than one ferrobridge tag default: $(printf '%s' "$tags" | tr '\n' ' ')"
  elif [ "$tags" != "$matrix_product" ]; then
    bad "quickstart tag: compose.yaml pulls $tags, docs/VERSIONS.md pins the product version $matrix_product"
  else
    note "OK: the quickstart pulls $tags"
    if [ -f Cargo.toml ]; then
      workspace_ver="$(toml_val "[workspace.package]" version Cargo.toml)"
      if [ -z "$workspace_ver" ]; then
        note "root Cargo.toml has no [workspace.package] version yet, skipped"
      elif [ "$tags" != "$workspace_ver" ]; then
        bad "quickstart tag: compose.yaml pulls $tags, root Cargo.toml is at $workspace_ver"
      else
        note "OK: the quickstart tag matches the workspace version"
      fi
    else
      note "no root Cargo.toml yet, skipped the quickstart tag comparison"
    fi
  fi

  cdm_image="$(sed -nE 's|^[[:space:]]*image:[[:space:]]*(postgres:[^[:space:]]+)[[:space:]]*$|\1|p' compose.yaml | head -n1)"
  want_pg="$(pin_of "PostgreSQL image" docs/VERSIONS.md)"
  if [ -z "$cdm_image" ]; then
    bad "compose.yaml has no digest-pinned postgres image"
  elif [ -z "$want_pg" ]; then
    bad "docs/VERSIONS.md has no 'PostgreSQL image' row"
  elif [ "$cdm_image" != "$want_pg" ]; then
    bad "CDM image: compose.yaml runs $cdm_image, docs/VERSIONS.md pins $want_pg"
  else
    note "OK: the CDM service runs $cdm_image"
  fi
else
  note "no compose.yaml yet, skipped"
fi

echo "== vendored corpora (docs/specs/*/PROVENANCE.md and the vendored fixtures <-> docs/VERSIONS.md)"
# A corpus row carries the repository and its commit or immutable tag in one
# cell, so this reads the whole cell rather than its first token.
pin_cell_of() {
  awk -F'|' -v item="$1" '
    NF >= 3 {
      k = $2; v = $3
      gsub(/`/, "", k); gsub(/`/, "", v)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", k)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
      if (k == item) { print v; exit }
    }
  ' "$2"
}

# The reference a pin cell names: its first 40-hex token, else the token after
# the word `tag`.
pinned_ref_of() {
  awk '{
    for (i = 1; i <= NF; i++) if ($i ~ /^[0-9a-f]{40}$/) { print $i; exit }
    for (i = 1; i < NF; i++) if ($i == "tag") { t = $(i + 1); gsub(/[,.;:]+$/, "", t); print t; exit }
  }' <<< "$1"
}

corpora="docs/specs/fhirconnect|FHIRconnect specification source
docs/specs/fhirconnect/draft-rest-api|FHIRconnect REST API chapter (draft, unmerged)
docs/specs/fhirconnect-mapping-lib|FHIRconnect mapping library (corpus, never an oracle)
docs/specs/omocl|OMOCL corpus
docs/specs/omop-cdm|OMOP CDM definitions and PostgreSQL DDL
docs/specs/its-rest|openEHR ITS-REST OpenAPI
tools/ferrobridge-testkit/fixtures/opt/kds|KDS Diagnose operational template (fixture)
tools/fhir-codegen/vendor/hl7-v2ig|HL7 v2 definitions (v2ig source of truth, never committed)
tools/fhir-codegen/vendor/hl7-v2ig|HL7 v2+ licence page"

if [ -f docs/VERSIONS.md ]; then
  agreed=0
  expected=0
  while IFS='|' read -r dir item; do
    [ -n "$dir" ] || continue
    expected=$((expected + 1))
    if [ ! -f "$dir/PROVENANCE.md" ]; then
      note "no $dir/PROVENANCE.md yet, skipped (run scripts/vendor/)"
      continue
    fi
    cell="$(pin_cell_of "$item" docs/VERSIONS.md)"
    want="$(pinned_ref_of "$cell")"
    if [ -z "$cell" ]; then
      bad "docs/VERSIONS.md has no '$item' row"
    elif [ -z "$want" ]; then
      bad "the docs/VERSIONS.md pin for '$item' names no commit and no tag"
    elif ! grep -qF "$want" "$dir/PROVENANCE.md"; then
      bad "$dir/PROVENANCE.md does not name the pin $want that docs/VERSIONS.md records for '$item'"
    else
      # A file hash or tree digest in the row is the proof of the bytes, so
      # the provenance has to carry each one too.
      missing=""
      while IFS= read -r hash; do
        [ -n "$hash" ] || continue
        grep -qF "$hash" "$dir/PROVENANCE.md" || missing="$missing $hash"
      done < <(grep -oE '\b[0-9a-f]{64}\b' <<< "$cell" || true)
      if [ -n "$missing" ]; then
        bad "$dir/PROVENANCE.md does not name the digest$missing that docs/VERSIONS.md records for '$item'"
      else
        agreed=$((agreed + 1))
      fi
    fi
  done <<< "$corpora"
  [ "$agreed" -eq "$expected" ] && note "OK: all $expected corpus provenance stamps name their pin"
else
  note "no docs/VERSIONS.md yet, skipped"
fi

echo "== FHIR packages (tools/fhir-codegen/vendor/*/PROVENANCE.md <-> docs/VERSIONS.md)"
if [ -f docs/VERSIONS.md ]; then
  # The rows of the FHIR-packages table: the package is the backticked name
  # opening the Item cell, the pin is the whole Pin cell.
  packages="$(awk -F'|' '
    /^## / { on = ($0 ~ /^## FHIR packages/); next }
    on && NF >= 4 && $2 ~ /^[[:space:]]*`/ {
      k = $2; v = $3
      sub(/^[[:space:]]*`/, "", k); sub(/`.*/, "", k)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
      if (k != "Item") print k "|" v
    }
  ' docs/VERSIONS.md)"
  agreed=0
  expected=0
  while IFS='|' read -r pkg ver; do
    [ -n "$pkg" ] || continue
    expected=$((expected + 1))
    prov="tools/fhir-codegen/vendor/$pkg/PROVENANCE.md"
    if [ ! -f "$prov" ]; then
      bad "no $prov for the pinned package $pkg (run scripts/vendor/fhir-packages.sh $pkg)"
    elif ! grep -qxF -- "- Version: $ver" "$prov"; then
      bad "$prov does not record version $ver, which docs/VERSIONS.md pins for $pkg"
    elif [ -d "tools/fhir-codegen/vendor/$pkg/repository" ] && ! grep -q '^- Repository licence: ' "$prov"; then
      bad "$prov vendors its source repository and records no repository licence"
    else
      agreed=$((agreed + 1))
    fi
  done <<< "$packages"
  if [ "$expected" -eq 0 ]; then
    bad "docs/VERSIONS.md has no FHIR-packages rows"
  elif [ "$agreed" -eq "$expected" ]; then
    note "OK: all $expected FHIR package provenance stamps record their pinned version"
  fi
else
  note "no docs/VERSIONS.md yet, skipped"
fi

echo "== licence (LICENSE <-> SPDX headers, manifests, badges, labels)"
if [ -f LICENSE ]; then
  stale=0
  if ! grep -q 'Business Source License 1.1' LICENSE; then
    bad "LICENSE is not the Business Source License 1.1"
    stale=1
  fi
  # crates/fhir-types is excepted: it is the one first-party crate under
  # Apache-2.0 (docs/architecture.md section 4.1), generated from the CC0 HL7
  # FHIR packages and published so any Rust project can depend on it.
  # crates/hl7v2-types is excepted beside it: the Rust fhir-codegen generates
  # from the HL7 v2 definitions is the project's own code under Apache-2.0
  # (owner ruling on #251), and the definitions are never committed. The SPDX
  # tag is anchored to the start of its line, after an optional comment marker,
  # so a header claim is caught while the same text quoted inside a string
  # literal (the emitter that writes that crate's header) is not.
  # The KDS project fixtures are modified copies of Apache-2.0 mapping-library files, so they keep that licence.
  # The omop-cdm era scripts are the PostgreSQL form of OHDSI's Apache-2.0 SQL, so they keep it too.
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    bad "stale licence claim at $hit"
    stale=1
  done < <(git grep -n -E '^[[:space:]]*([/#*]+|<!--)?[[:space:]]*SPDX-License-Identifier: (MIT|Apache-2\.0)|License-MIT|License-Apache|^license = "(MIT|Apache-2\.0)"|^license: (MIT|Apache-2\.0)|image\.licenses="?(MIT|Apache)' \
    -- ':!LICENSE' ':!CHANGELOG.md' ':!scripts/checks/versions.sh' ':(glob,exclude)**/vendor/**' \
    ':(glob,exclude)docs/specs/**' ':(glob,exclude)crates/fhir-types/**' ':(glob,exclude)crates/hl7v2-types/**' \
    ':(glob,exclude)crates/fhirconnect/tests/fixtures/projects/ferrobridge/kds_diagnose/**' \
    ':(glob,exclude)crates/omop-cdm/sql/*.sql' || true)
  [ "$stale" -eq 0 ] && note "OK: every first-party file names BUSL-1.1 (crates/fhir-types, crates/hl7v2-types, the KDS project fixtures and the omop-cdm era scripts excepted, Apache-2.0)"
else
  note "no LICENSE yet, skipped"
fi

echo
if [ "$fail" -ne 0 ]; then
  echo "versions: DRIFT detected" >&2
  exit 1
fi
echo "versions: OK (every present check agrees)"
