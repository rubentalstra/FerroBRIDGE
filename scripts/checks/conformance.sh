#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# The conformance gate (#24): runs the six corpus tests, compares what they
# measured with the committed pass lists under conformance/, and renders one
# shields.io endpoint badge per corpus (https://shields.io/badges/endpoint-badge).
#
#   scripts/checks/conformance.sh            # run and compare; warn on drift
#   scripts/checks/conformance.sh --update   # run and rewrite lists and badges
#   scripts/checks/conformance.sh --check    # run and fail on any drift (CI)
#
# The tests write target/conformance/<corpus>.json through the testkit's
# conformance module, and rewrite the pass lists themselves when
# FERROBRIDGE_CONFORMANCE_UPDATE is 1, so the lists come from the same tests
# the suite runs. A case the list records that no longer passes fails every
# mode. Under --check, an unlisted passing case, a moved total, a badge that
# disagrees with its list, and a README badge naming no committed badge fail
# too, because the list is what the ratchet protects.
#
# The hl7v2-smoke corpus reads the NIST and AIRA sets, which it fetches first
# through scripts/vendor/hl7v2-samples.sh --build-time (a no-op when they are
# on disk at their pins).
#
# Needs cargo, cargo-nextest, jq, and the tools that script needs. Exit 0 when
# clean, 1 on a regression, (under --check) drift or no cargo on PATH, 2 on a
# usage error, a missing result or a failed fetch.
set -euo pipefail

# A missing cargo would otherwise read as every corpus test failing.
if ! command -v cargo >/dev/null 2>&1; then
  echo "conformance: cargo not found on PATH" >&2
  exit 1
fi
cd "$(dirname "$0")/../.."

readonly CORPORA=(fhirconnect-mapping-lib omocl roundtrip draft-rest-api hl7v2 hl7v2-smoke)
readonly OUT=target/conformance
readonly BADGES=conformance/badges

mode=compare
case "${1:-}" in
  "") ;;
  --update) mode=update ;;
  --check) mode=check ;;
  *)
    echo "usage: $0 [--update|--check]" >&2
    exit 2
    ;;
esac

# The badge label of a corpus.
label_of() {
  case "$1" in
    fhirconnect-mapping-lib) echo "FHIRconnect mapping library" ;;
    omocl) echo "OMOCL mapping library" ;;
    roundtrip) echo "FHIR round-trip laws" ;;
    draft-rest-api) echo "FHIRconnect REST API (draft)" ;;
    hl7v2) echo "HL7 v2 message corpora" ;;
    hl7v2-smoke) echo "HL7 v2 smoke corpora (NIST, AIRA)" ;;
    *) return 1 ;;
  esac
}

# The badge colour of k passing out of n: the share in quarters.
colour_of() {
  local passed="$1" total="$2" share
  if [[ "$total" -eq 0 ]]; then
    echo lightgrey
    return
  fi
  share=$((passed * 100 / total))
  if [[ "$passed" -eq "$total" ]]; then echo brightgreen
  elif [[ "$share" -ge 75 ]]; then echo green
  elif [[ "$share" -ge 50 ]]; then echo yellow
  elif [[ "$share" -ge 25 ]]; then echo orange
  else echo red
  fi
}

# The badge JSON a pass list implies.
badge_of() {
  local corpus="$1" list="conformance/$1/pass-list.txt" passed total
  passed="$(grep -vc '^total ' "$list" || true)"
  total="$(sed -n 's/^total \([0-9][0-9]*\)$/\1/p' "$list")"
  [[ -n "$total" ]] || { echo "conformance: $list has no total line" >&2; return 2; }
  jq -cn --arg label "$(label_of "$corpus")" --arg message "$passed / $total" \
    --arg color "$(colour_of "$passed" "$total")" \
    '{schemaVersion: 1, label: $label, message: $message, color: $color}'
}

failed=0

rm -rf "$OUT"
export FERROBRIDGE_CONFORMANCE_OUT="$PWD/$OUT"
if [[ "$mode" = update ]]; then
  export FERROBRIDGE_CONFORMANCE_UPDATE=1
fi
# The hl7v2-smoke corpus reads the HL7 v2 sets fetched at build time; the
# script keeps a tree already at its pinned digest without a download.
if ! scripts/vendor/hl7v2-samples.sh --build-time; then
  echo "conformance: the build-time HL7 v2 sets could not be fetched" >&2
  exit 2
fi
# One package per invocation, as the CI test lane runs them.
for package in fhirconnect omocl ferrobridge-hl7v2; do
  if ! cargo nextest run --locked -p "$package" --no-tests=fail --no-fail-fast \
    -E 'test(/^[a-z_]+::conformance_/)'; then
    echo "conformance: a $package corpus test failed; a case its list records no longer passes"
    failed=1
  fi
done

for corpus in "${CORPORA[@]}"; do
  result="$OUT/$corpus.json"
  list="conformance/$corpus/pass-list.txt"
  if [[ ! -f "$result" ]]; then
    echo "conformance: no test reported $corpus ($result is missing)" >&2
    exit 2
  fi
  passing="$(jq -r '.cases[] | select(.passed) | .id' "$result" | LC_ALL=C sort)"
  measured_total="$(jq -r '.total' "$result")"
  listed="$(grep -v '^total ' "$list" | LC_ALL=C sort || true)"
  listed_total="$(sed -n 's/^total \([0-9][0-9]*\)$/\1/p' "$list")"
  regressed="$(LC_ALL=C comm -23 <(printf '%s\n' "$listed" | sed '/^$/d') <(printf '%s\n' "$passing" | sed '/^$/d'))"
  new="$(LC_ALL=C comm -13 <(printf '%s\n' "$listed" | sed '/^$/d') <(printf '%s\n' "$passing" | sed '/^$/d'))"
  count="$(printf '%s\n' "$passing" | sed '/^$/d' | wc -l | tr -d ' ')"
  echo "conformance: $corpus $count / $measured_total"
  if [[ -n "$regressed" ]]; then
    echo "conformance: $corpus regressed:"
    while IFS= read -r id; do
      echo "  $id: $(jq -r --arg id "$id" '(.cases[] | select(.id == $id) | .failure) // "the corpus no longer holds it"' "$result")"
    done <<<"$regressed"
    failed=1
  fi
  if [[ -n "$new" ]]; then
    echo "conformance: $corpus passes cases its list does not record; run $0 --update:"
    printf '  %s\n' "$new"
    if [[ "$mode" = check ]]; then failed=1; fi
  fi
  if [[ "$measured_total" != "$listed_total" ]]; then
    echo "conformance: $corpus holds $measured_total cases and its list records $listed_total; run $0 --update"
    if [[ "$mode" = check ]]; then failed=1; fi
  fi

  badge="$BADGES/$corpus.json"
  rendered="$(badge_of "$corpus")"
  if [[ "$mode" = update ]]; then
    mkdir -p "$BADGES"
    printf '%s\n' "$rendered" >"$badge"
  elif [[ ! -f "$badge" ]] || [[ "$(cat "$badge")" != "$rendered" ]]; then
    echo "conformance: $badge disagrees with $list; run $0 --update"
    if [[ "$mode" = check ]]; then failed=1; fi
  fi
done

# Every conformance badge the README shows names a committed badge file, and
# every corpus has one on the README.
readme_badges="$(grep -oE 'conformance%2Fbadges%2F[a-z0-9-]+\.json' README.md | sed 's/.*%2F//' || true)"
for corpus in "${CORPORA[@]}"; do
  if ! grep -qx "$corpus.json" <<<"$readme_badges"; then
    echo "conformance: README.md shows no badge for $corpus"
    failed=1
  fi
done
while IFS= read -r named; do
  [[ -n "$named" ]] || continue
  if [[ ! -f "$BADGES/$named" ]]; then
    echo "conformance: README.md names $BADGES/$named, which does not exist"
    failed=1
  fi
done <<<"$readme_badges"

exit "$failed"
