#!/usr/bin/env bash
# SPDX-FileCopyrightText: Ruben Talstra
# SPDX-License-Identifier: BUSL-1.1
# The crates.io upload of the `crates/*` members, and its read-back, as one
# implementation shared by the `crates` leg of release.yml (a `v*` tag,
# approval-gated) and publish-crates.yml (a manual dispatch: the dry-run and
# recovery lane). The two lanes cannot share a reusable workflow: crates.io
# Trusted Publishing matches the OIDC `workflow_ref` claim, and for a job inside
# a reusable workflow that claim names the calling workflow
# (<https://crates.io/docs/trusted-publishing>), so the shared thing is this
# script and each lane keeps its own job and identity.
#
# Per crate, in dependency order, because `cargo publish --workspace` refuses
# the whole run when any member version already exists while being non-atomic
# at the end: a partial publish could not be finished by re-running it. One
# crate at a time, "already exists" counted as done, makes the lane resumable
# and idempotent; the registry is read back before success is reported.
#
# Each crate is published and verified at the version in its OWN manifest.
# `fhir-types` carries the crate line (docs/VERSIONS.md) and every other member
# still holds its name at the 0.0.0 placeholder, so the set is deliberately not
# lockstep today.
#
# Usage:
#   publish-crates.sh publish          # upload each crate in dependency order
#   publish-crates.sh verify           # read the registry back, with retries
#   publish-crates.sh version [CRATE]  # print one crate's manifest version
#
# Requires cargo, curl, and jq; `publish` also needs CARGO_REGISTRY_TOKEN.
set -euo pipefail
cd "$(dirname "$0")/../.."

# Dependency order: a crate is uploaded only after every sibling it depends on
# is on the index.
readonly CRATES=(
  fhir-types
  openehr-mapping-core
  fhirconnect
  omocl
  omop-cdm
  ferrobridge-openehr
  ferrobridge-term
)

# The `[package]` table's own `version` in one member's manifest.
manifest_version() {
  awk -F'"' '/^\[package\]/{p=1} p && /^version = /{print $2; exit}' "crates/$1/Cargo.toml"
}

# cargo colours the status word, so "Uploaded" is followed by a reset sequence
# before the crate name; strip the colour before matching.
readonly ESC=$'\033'
strip_ansi() {
  sed -E "s/${ESC}\\[[0-9;]*m//g"
}

do_publish() {
  local crate out plain failed=""
  for crate in "${CRATES[@]}"; do
    echo "::group::$crate $(manifest_version "$crate")"
    out="$(cargo publish -p "$crate" --locked 2>&1)" || true
    printf '%s\n' "$out"
    echo "::endgroup::"
    plain="$(printf '%s' "$out" | strip_ansi)"
    case "$plain" in
    *"already exists on crates.io index"* | *"already uploaded"*)
      echo "$crate: already published at this version, nothing to do"
      ;;
    *)
      if printf '%s' "$plain" | grep -q "Uploaded $crate"; then
        echo "$crate: uploaded"
      else
        failed="$failed $crate"
      fi
      ;;
    esac
  done
  [[ -z "$failed" ]] || {
    echo "::error::failed to publish:$failed"
    return 1
  }
  echo "publish-crates: every crate is at its manifest version or was already there"
}

# A half-published set is worse than an unpublished one: while a line is 0.x,
# cargo treats every 0.x as its own compatibility set, so one straggler makes
# its siblings' internal requirements unresolvable. Read the registry, never
# the exit code alone.
do_verify() {
  local crate want got body bad=""
  for crate in "${CRATES[@]}"; do
    want="$(manifest_version "$crate")"
    # The index is eventually consistent right after an upload: a miss is
    # retried, and a failed request counts as "not seen yet", not as a miss.
    got=""
    for _ in 1 2 3 4 5 6; do
      if body="$(curl --proto '=https' --tlsv1.2 -sSL --fail \
        -H 'User-Agent: ferrobridge-publish-verify (scripts/release/publish-crates.sh)' \
        "https://crates.io/api/v1/crates/$crate/versions" 2>/dev/null)"; then
        got="$(printf '%s' "$body" |
          jq -r --arg v "$want" '.versions[]? | select(.num == $v) | .num' |
          head -1)" || got=""
      fi
      [[ -n "$got" ]] && break
      sleep 10
    done
    printf '%-22s want %-10s %s\n' "$crate" "$want" "${got:-MISSING}"
    [[ -n "$got" ]] || bad="$bad $crate@$want"
  done
  [[ -z "$bad" ]] || {
    echo "::error::the published set is incomplete:$bad"
    return 1
  }
  echo "publish-crates: confirmed on crates.io, all ${#CRATES[@]} crates at their manifest versions"
}

case "${1:-}" in
publish) do_publish ;;
verify) do_verify ;;
version) manifest_version "${2:-fhir-types}" ;;
*)
  echo "publish-crates: expected 'publish', 'verify', or 'version', got '${1:-<none>}'" >&2
  exit 2
  ;;
esac
