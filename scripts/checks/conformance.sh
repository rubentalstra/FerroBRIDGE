#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# The conformance gate (#24): runs the six corpus tests, compares what they
# measured with the committed pass lists under conformance/, and renders one
# shields.io endpoint badge per corpus (https://shields.io/badges/endpoint-badge),
# one per HL7 v2 message family and one per HL7 v2 version (#366), and the
# README block between badges:begin and badges:end from those badges.
#
#   scripts/checks/conformance.sh            # run and compare; warn on drift
#   scripts/checks/conformance.sh --update   # run and rewrite lists, badges, README block
#   scripts/checks/conformance.sh --check    # run and fail on any drift (CI)
#
# The tests write target/conformance/<corpus>.json through the testkit's
# conformance module, and rewrite the pass lists themselves when
# FERROBRIDGE_CONFORMANCE_UPDATE is 1, so the lists come from the same tests
# the suite runs. A case the list records that no longer passes fails every
# mode. Under --check, an unlisted passing case, a moved total, a badge that
# disagrees with what --update would write, a family or version badge no case
# counts, and a README block that disagrees with the badges fail too, because
# the list is what the ratchet protects.
#
# The family and version badges count the cases of both HL7 v2 corpora by the
# family (MSH-9.1) and the version (MSH-12) the corpus test records per case:
# one badge per family with at least one case, by family code, and one per
# version seen. They link to the hl7v2 pass list.
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
# The shields.io endpoint prefix every conformance badge file is read through.
readonly ENDPOINT='https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Frubentalstra%2FFerroBRIDGE%2Fmain%2Fconformance%2Fbadges%2F'
# The build badges, the first row of the README block.
readonly BUILD_BADGES=(
  '[![CI](https://github.com/rubentalstra/FerroBRIDGE/actions/workflows/ci.yml/badge.svg)](https://github.com/rubentalstra/FerroBRIDGE/actions/workflows/ci.yml)'
  '[![CodeQL](https://github.com/rubentalstra/FerroBRIDGE/actions/workflows/codeql.yml/badge.svg)](https://github.com/rubentalstra/FerroBRIDGE/actions/workflows/codeql.yml)'
  '[![OpenSSF Scorecard](https://api.securityscorecards.dev/projects/github.com/rubentalstra/FerroBRIDGE/badge)](https://scorecard.dev/viewer/?uri=github.com/rubentalstra/FerroBRIDGE)'
  '[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=rubentalstra_FerroBRIDGE&metric=alert_status)](https://sonarcloud.io/summary/overall?id=rubentalstra_FerroBRIDGE)'
  '[![Coverage](https://sonarcloud.io/api/project_badges/measure?project=rubentalstra_FerroBRIDGE&metric=coverage)](https://sonarcloud.io/summary/new_code?id=rubentalstra_FerroBRIDGE)'
  '[![License: BUSL-1.1](https://img.shields.io/badge/License-BUSL--1.1-blue.svg)](LICENSE)'
  '[![GitHub release (latest SemVer)](https://img.shields.io/github/v/release/rubentalstra/FerroBRIDGE?sort=semver)](https://github.com/rubentalstra/FerroBRIDGE/releases/latest)'
)

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

# The badge JSON of a label and k passing out of n.
fraction_badge() {
  jq -cn --arg label "$1" --arg message "$2 / $3" --arg color "$(colour_of "$2" "$3")" \
    '{schemaVersion: 1, label: $label, message: $message, color: $color}'
}

# The badge JSON a pass list implies.
badge_of() {
  local corpus="$1" list="conformance/$1/pass-list.txt" passed total
  passed="$(grep -vc '^total ' "$list" || true)"
  total="$(sed -n 's/^total \([0-9][0-9]*\)$/\1/p' "$list")"
  [[ -n "$total" ]] || { echo "conformance: $list has no total line" >&2; return 2; }
  fraction_badge "$(label_of "$corpus")" "$passed" "$total"
}

failed=0

# Writes badge $1 as $2 under --update, else compares it with the file and
# names $3, what the badge is rendered from, when they disagree.
emit_badge() {
  local badge="$BADGES/$1.json"
  if [[ "$mode" = update ]]; then
    mkdir -p "$BADGES"
    printf '%s\n' "$2" >"$badge"
  elif [[ ! -f "$badge" ]] || [[ "$(cat "$badge")" != "$2" ]]; then
    echo "conformance: $badge disagrees with $3; run $0 --update"
    if [[ "$mode" = check ]]; then failed=1; fi
  fi
}

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
    -E 'test(/conformance_/)'; then
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
  emit_badge "$corpus" "$(badge_of "$corpus")" "$list"
done

# Every case of both HL7 v2 corpora, and the passing and total count of the
# cases whose field $1 is $2.
hl7v2_cases="$(jq -c -s '[.[].cases[]]' "$OUT/hl7v2.json" "$OUT/hl7v2-smoke.json")"
fraction_of() {
  jq -r --arg field "$1" --arg value "$2" \
    '[.[] | select(.[$field] == $value)] | "\([.[] | select(.passed)] | length) \(length)"' \
    <<<"$hl7v2_cases"
}

families=()
while IFS= read -r family; do
  [[ -n "$family" ]] || continue
  read -r passed total < <(fraction_of family "$family")
  name="hl7v2-family-$(tr '[:upper:]' '[:lower:]' <<<"$family")"
  families+=("$name")
  emit_badge "$name" "$(fraction_badge "$family" "$passed" "$total")" "the HL7 v2 results"
done < <(jq -r '[.[] | .family | select(. != null and test("^[A-Z0-9]+$"))] | unique | .[]' \
  <<<"$hl7v2_cases" | LC_ALL=C sort)
versions=()
while IFS= read -r version; do
  [[ -n "$version" ]] || continue
  read -r passed total < <(fraction_of version "$version")
  name="hl7v2-version-$version"
  versions+=("$name")
  emit_badge "$name" "$(fraction_badge "v$version" "$passed" "$total")" "the HL7 v2 results"
done < <(jq -r '[.[] | .version | select(. != null and test("^[0-9]+(\\.[0-9]+)+$"))]
  | unique | sort_by(split(".") | map(tonumber)) | .[]' <<<"$hl7v2_cases")

# A family or version badge no case counts any more goes with --update.
derived=" ${families[*]-} ${versions[*]-} "
for stale in "$BADGES"/hl7v2-family-*.json "$BADGES"/hl7v2-version-*.json; do
  [[ -f "$stale" ]] || continue
  if [[ "$derived" != *" $(basename "$stale" .json) "* ]]; then
    if [[ "$mode" = update ]]; then
      rm "$stale"
    else
      echo "conformance: $stale counts no case any more; run $0 --update"
      if [[ "$mode" = check ]]; then failed=1; fi
    fi
  fi
done

# The markdown of one badge: its alt text, its badge file, the list it links to.
badge_link() {
  echo "[![$1](${ENDPOINT}${2}.json)]($3)"
}

# The README block with its markers, in a fixed order: the build badges, then
# one row per standard, the HL7 v2 row closing on its family and version badges.
render_block() {
  local name
  echo '<!-- badges:begin -->'
  printf '%s\n' "${BUILD_BADGES[@]}"
  echo
  echo "**Conformance**, measured by the corpus tests under \`conformance/\`; each badge links to its pass list."
  echo
  echo 'FHIRconnect 1.0.0:'
  badge_link "$(label_of fhirconnect-mapping-lib)" fhirconnect-mapping-lib conformance/fhirconnect-mapping-lib/pass-list.txt
  badge_link "$(label_of draft-rest-api)" draft-rest-api conformance/draft-rest-api/pass-list.txt
  echo
  echo 'OMOCL 1.0.0:'
  badge_link "$(label_of omocl)" omocl conformance/omocl/pass-list.txt
  echo
  echo 'FHIR R4:'
  badge_link "$(label_of roundtrip)" roundtrip conformance/roundtrip/pass-list.txt
  echo
  echo 'HL7 v2, by corpus, then by message family and by version across both corpora:'
  badge_link "$(label_of hl7v2)" hl7v2 conformance/hl7v2/pass-list.txt
  badge_link "$(label_of hl7v2-smoke)" hl7v2-smoke conformance/hl7v2-smoke/pass-list.txt
  for name in ${families[@]+"${families[@]}"}; do
    badge_link "HL7 v2 $(tr '[:lower:]' '[:upper:]' <<<"${name#hl7v2-family-}")" "$name" conformance/hl7v2/pass-list.txt
  done
  for name in ${versions[@]+"${versions[@]}"}; do
    badge_link "HL7 v${name#hl7v2-version-}" "$name" conformance/hl7v2/pass-list.txt
  done
  echo '<!-- badges:end -->'
}

block="$(render_block)"
if ! grep -qx '<!-- badges:begin -->' README.md || ! grep -qx '<!-- badges:end -->' README.md; then
  echo "conformance: README.md has no badges:begin and badges:end markers"
  exit 1
fi
if [[ "$mode" = update ]]; then
  rendered="$(mktemp)"
  trap 'rm -f "$rendered" "$rendered.readme"' EXIT
  printf '%s\n' "$block" >"$rendered"
  awk -v block="$rendered" '
    $0 == "<!-- badges:begin -->" { while ((getline line < block) > 0) print line; skipping = 1; next }
    $0 == "<!-- badges:end -->" { skipping = 0; next }
    !skipping { print }
  ' README.md >"$rendered.readme"
  cat "$rendered.readme" >README.md
elif [[ "$(sed -n '/^<!-- badges:begin -->$/,/^<!-- badges:end -->$/p' README.md)" != "$block" ]]; then
  echo "conformance: the README.md badge block disagrees with the badge files; run $0 --update"
  if [[ "$mode" = check ]]; then failed=1; fi
fi

exit "$failed"
