#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/hl7v2-samples.sh
#
# The HL7 v2 message corpora the corpus test of crates/ferrobridge-hl7v2 reads
# (#292), under crates/ferrobridge-hl7v2/vendor/, which the crate's `include`
# keeps out of the published package.
#
# Three sets are vendored verbatim and committed, each with a PROVENANCE.md and
# its upstream LICENSE: the Microsoft FHIR-Converter samples with their
# expected R4 Bundles (MIT), the CDC ReportStream data tests (CC0-1.0), and the
# HL7 v2-to-FHIR benchmark page with its sample Bundles (Apache-2.0), from
# whose markdown the seven benchmark messages are extracted into derived files.
#
# Four sets are fetched at build time into ignored directories, only their
# PROVENANCE.md committed, as scripts/vendor/v2ig.sh does: the NIST LRI, LOI
# and syndromic surveillance test bundles, and a deterministic part of the AIRA
# MQE example messages. Neither repository carries a licence file.
#
# Every pin, file count and tree digest is a row of docs/VERSIONS.md. A tree
# already on disk at its pinned count and digest is kept without a download.
#
# Usage:
#   scripts/vendor/hl7v2-samples.sh                        # verify or fetch the vendored sets
#   scripts/vendor/hl7v2-samples.sh --stamp                # after moving a pin: fetch, rewrite
#                                                          # each PROVENANCE.md, print the count
#                                                          # and digest for the docs/VERSIONS.md row
#   scripts/vendor/hl7v2-samples.sh --build-time           # verify or fetch the build-time sets
#   scripts/vendor/hl7v2-samples.sh --build-time --stamp   # the same stamp for the build-time sets
#   scripts/vendor/hl7v2-samples.sh --cache-key            # a CI cache key over the build-time pins
#
# Requires: curl, tar, shasum, jq, git.
#
# No specification governs this script; it is FerroBRIDGE's own design.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

usage="usage: scripts/vendor/hl7v2-samples.sh [--build-time] [--stamp] | --cache-key"
scope=vendored
stamp=0
for arg in "$@"; do
  case "$arg" in
    --build-time) scope=build-time ;;
    --stamp) stamp=1 ;;
    --cache-key) scope=cache-key ;;
    *) die "$usage" ;;
  esac
done

corpus_require curl tar shasum jq git

base="crates/ferrobridge-hl7v2/vendor"

# Every row this script reads, by its Item cell in docs/VERSIONS.md.
item_converter="HL7 v2 samples: Microsoft FHIR-Converter"
item_reportstream="HL7 v2 samples: CDC ReportStream data tests"
item_v2tofhir="HL7 v2 samples: HL7 v2-to-FHIR benchmark messages"
item_lri="HL7 v2 samples: NIST LRI (build time, never committed)"
item_loi="HL7 v2 samples: NIST LOI (build time, never committed)"
item_ss="HL7 v2 samples: NIST syndromic surveillance (build time, never committed)"
item_aira="HL7 v2 samples: AIRA MQE (build time, never committed)"

# The token after the word $1 in the pin cell $2.
pin_field() {
  awk -v key="$1" '{ for (i = 1; i < NF; i++) if ($i == key) { print $(i + 1); exit } }' <<< "$2"
}

if [ "$scope" = "cache-key" ]; then
  keyed=""
  for item in "$item_lri" "$item_loi" "$item_ss" "$item_aira"; do
    keyed="$keyed$(corpus_pin_cell "$item")
"
  done
  printf '%s' "$keyed" | shasum -a 256 | cut -c1-16
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# The file count and tree digest of the tree $1, without PROVENANCE.md.
tree_state() {
  if [ -d "$1" ]; then
    printf '%s %s\n' "$(corpus_file_count "$1")" "$(corpus_tree_digest "$1")"
  fi
}

# Answers whether the tree $1 already holds the row $2 at its pinned count and
# digest, which a fetch then keeps.
at_pin() {
  local cell want_files want_digest
  cell="$(corpus_pin_cell "$2")"
  want_files="$(pin_field files "$cell")"
  want_digest="$(pin_field digest "$cell")"
  grep -qE '^[0-9]+$' <<< "$want_files" || die "the '$2' pin names no file count"
  grep -qE '^[0-9a-f]{64}$' <<< "$want_digest" || die "the '$2' pin names no tree digest"
  [ "$(tree_state "$1")" = "$want_files $want_digest" ]
}

# Checks the staged tree $1 against the row $2 and the provenance $3, unless a
# stamp is rewriting both.
verify_stage() {
  local cell files digest
  [ "$stamp" -eq 0 ] || return 0
  cell="$(corpus_pin_cell "$2")"
  read -r files digest <<< "$(tree_state "$1")"
  [ "$files" = "$(pin_field files "$cell")" ] || die "'$2' stages $files files, the pin records $(pin_field files "$cell")"
  [ "$digest" = "$(pin_field digest "$cell")" ] || die "'$2' stages the digest $digest, the pin records $(pin_field digest "$cell")"
  [ -f "$3" ] || die "no $3 names the pin; run with --stamp"
  grep -qF "$(corpus_pin_commit "$cell")" "$3" || die "$3 does not name the pinned commit"
  grep -qF "$digest" "$3" || die "$3 does not name the digest $digest"
}

# Replaces everything in $2 except its PROVENANCE.md by the staged tree $1.
install_stage() {
  mkdir -p "$2"
  find "$2" -mindepth 1 -maxdepth 1 ! -name PROVENANCE.md -exec rm -rf {} +
  find "$1" -mindepth 1 -maxdepth 1 -exec mv {} "$2"/ \;
}

# Prints what a stamp records for the row $2 over the tree $1.
report() {
  local files digest
  read -r files digest <<< "$(tree_state "$1")"
  say "$2: $files files, tree digest $digest"
  [ "$stamp" -eq 0 ] || say "record in the '$2' row of docs/VERSIONS.md: files $files digest $digest"
}

# Extracts the benchmark messages of test_conversions.md ($1) into $2, one file
# per `#### <structure>` section, holding the lines between its `<tr>` and
# `</tr>` with the carriage returns, the `<br>` openers and the empty lines
# removed, one segment per line.
extract_benchmarks() {
  mkdir -p "$2"
  awk -v out="$2" '
    { sub(/\r$/, "") }
    /^#### / { name = $2; next }
    /^<tr>$/ { if (name != "") { file = out "/" name ".hl7"; inside = 1; printf "" > file }; next }
    /^<\/tr>$/ { if (inside) close(file); inside = 0; name = ""; next }
    inside {
      line = $0
      sub(/^<br>/, "", line)
      if (line != "") print line >> file
    }
  ' "$1"
}

vendor_converter() {
  local dest="$base/fhir-converter" pin repo commit tree_root stage="$tmp/converter"
  if [ "$stamp" -eq 0 ] && at_pin "$dest" "$item_converter"; then
    report "$dest" "$item_converter"
    return
  fi
  pin="$(corpus_pin_cell "$item_converter")"
  repo="$(corpus_pin_repo "$pin")"
  commit="$(corpus_pin_commit "$pin")"
  say "$repo at $commit"
  mkdir -p "$tmp/converter-fetch" "$stage"
  tree_root="$(corpus_fetch "$repo" "$commit" "$tmp/converter-fetch")"
  corpus_take "$tree_root" "$stage" LICENSE data/SampleData/Hl7v2 \
    src/Microsoft.Health.Fhir.Liquid.Converter.FunctionalTests/TestData/Expected/Hl7v2
  rm -rf "$tmp/converter-fetch"
  verify_stage "$stage" "$item_converter" "$dest/PROVENANCE.md"
  install_stage "$stage" "$dest"
  if [ "$stamp" -eq 1 ]; then
    local files digest messages expected
    read -r files digest <<< "$(tree_state "$dest")"
    messages="$(find "$dest/data" -type f -name '*.hl7' | wc -l | tr -d '[:space:]')"
    expected="$(find "$dest/src" -type f -name '*-expected.json' | wc -l | tr -d '[:space:]')"
    cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the Microsoft FHIR-Converter HL7 v2 samples

Two directories of the repository, vendored verbatim at their upstream layout
by \`scripts/vendor/hl7v2-samples.sh\` (.claude/rules/vendored-inputs.md).
Never edit a file here: change the pin in docs/VERSIONS.md, run the script
with \`--stamp\`, and record the count and digest it prints.

- Source: <https://github.com/$repo>
- Pin: commit \`$commit\`
- Paths: \`data/SampleData/Hl7v2\` ($messages HL7 v2 messages) and
  \`src/Microsoft.Health.Fhir.Liquid.Converter.FunctionalTests/TestData/Expected/Hl7v2\`
  ($expected expected R4 Bundles, one per message it names, under
  \`<type>_<event>/<message>-expected.json\`)
- Upstream licence: MIT, the repository's \`LICENSE\` file, vendored beside
  this file
- Files: $files
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`
- Stamped: $(corpus_fetched)

The messages are test messages the converter's authors wrote, never patient
data. The expected Bundles are what Microsoft's own Liquid templates produce,
so the corpus test reads them as a comparison, never as the v2-to-FHIR
guide's answer: every difference is a counted outcome.
PROV
  fi
  report "$dest" "$item_converter"
}

vendor_reportstream() {
  local dest="$base/reportstream" pin repo commit tree_root stage="$tmp/reportstream"
  if [ "$stamp" -eq 0 ] && at_pin "$dest" "$item_reportstream"; then
    report "$dest" "$item_reportstream"
    return
  fi
  pin="$(corpus_pin_cell "$item_reportstream")"
  repo="$(corpus_pin_repo "$pin")"
  commit="$(corpus_pin_commit "$pin")"
  say "$repo at $commit"
  mkdir -p "$tmp/reportstream-fetch" "$stage"
  tree_root="$(corpus_fetch "$repo" "$commit" "$tmp/reportstream-fetch")"
  corpus_take "$tree_root" "$stage" LICENSE prime-router/src/testIntegration/resources/datatests
  rm -rf "$tmp/reportstream-fetch"
  verify_stage "$stage" "$item_reportstream" "$dest/PROVENANCE.md"
  install_stage "$stage" "$dest"
  if [ "$stamp" -eq 1 ]; then
    local files digest messages bundles
    read -r files digest <<< "$(tree_state "$dest")"
    messages="$(find "$dest/prime-router" -type f -name '*.hl7' | wc -l | tr -d '[:space:]')"
    bundles="$(find "$dest/prime-router" -type f -name '*.fhir' | wc -l | tr -d '[:space:]')"
    cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the CDC ReportStream data tests

One directory of the repository, vendored verbatim at its upstream layout by
\`scripts/vendor/hl7v2-samples.sh\` (.claude/rules/vendored-inputs.md). Never
edit a file here: change the pin in docs/VERSIONS.md, run the script with
\`--stamp\`, and record the count and digest it prints.

- Source: <https://github.com/$repo> (archived by its owner)
- Pin: commit \`$commit\`
- Path: \`prime-router/src/testIntegration/resources/datatests\`: $messages
  \`.hl7\` files (single messages and \`FHS\`/\`BHS\` batches) and $bundles
  \`.fhir\` R4 Bundles, with the CSV inputs and the test configuration
- Upstream licence: CC0-1.0, the repository's \`LICENSE\` file, vendored
  beside this file
- Files: $files
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`
- Stamped: $(corpus_fetched)

The messages are test messages ReportStream's authors wrote, never patient
data. In \`HL7_to_FHIR/\` and \`mappinginventory/\` a \`.fhir\` Bundle beside a
\`.hl7\` message is ReportStream's conversion of it, which the corpus test
reads as a comparison, never as the v2-to-FHIR guide's answer. In
\`FHIR_to_HL7/\` the direction is the other way, so no Bundle there is read as
an expected result.
PROV
  fi
  report "$dest" "$item_reportstream"
}

vendor_v2tofhir() {
  local dest="$base/v2-to-fhir" pin repo commit tree_root stage="$tmp/v2-to-fhir"
  if [ "$stamp" -eq 0 ] && at_pin "$dest" "$item_v2tofhir"; then
    report "$dest" "$item_v2tofhir"
    return
  fi
  pin="$(corpus_pin_cell "$item_v2tofhir")"
  repo="$(corpus_pin_repo "$pin")"
  commit="$(corpus_pin_commit "$pin")"
  say "$repo at $commit"
  mkdir -p "$tmp/v2-to-fhir-fetch" "$stage"
  tree_root="$(corpus_fetch "$repo" "$commit" "$tmp/v2-to-fhir-fetch")"
  corpus_take "$tree_root" "$stage" LICENSE input/pagecontent/test_conversions.md samples
  rm -rf "$tmp/v2-to-fhir-fetch"
  extract_benchmarks "$stage/input/pagecontent/test_conversions.md" "$stage/derived"
  verify_stage "$stage" "$item_v2tofhir" "$dest/PROVENANCE.md"
  install_stage "$stage" "$dest"
  if [ "$stamp" -eq 1 ]; then
    local files digest derived
    read -r files digest <<< "$(tree_state "$dest")"
    derived=""
    while IFS= read -r name; do
      derived="$derived- \`derived/$name\`
"
    done < <(find "$dest/derived" -type f -name '*.hl7' -exec basename {} \; | LC_ALL=C sort)
    derived="${derived%$'\n'}"
    cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the HL7 v2-to-FHIR benchmark messages

Two paths of the repository, vendored verbatim at their upstream layout by
\`scripts/vendor/hl7v2-samples.sh\` (.claude/rules/vendored-inputs.md), and
the files the script derives from one of them. Never edit a file here: change
the pin in docs/VERSIONS.md, run the script with \`--stamp\`, and record the
count and digest it prints.

- Source: <https://github.com/$repo>
- Pin: commit \`$commit\`
- Paths: \`input/pagecontent/test_conversions.md\` (the benchmark page) and
  \`samples\` (HL7-authored R4 Bundles for ADT_A01 and MDM_T02, and the
  MDM_T02 message beside the second)
- Upstream licence: Apache License 2.0, the repository's \`LICENSE\` file,
  vendored beside this file
- Files: $files, the derived files included
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`
- Stamped: $(corpus_fetched)

## Derived files

The page embeds each benchmark message in HTML, one segment per line inside a
\`<tr>\` element. The script writes each into its own file under \`derived/\`,
named by the \`####\` heading of its section: the lines between \`<tr>\` and
\`</tr>\`, with each line's carriage return and \`<br>\` opener removed and the
empty lines dropped, one segment per line, bytes otherwise unchanged. These
files modify \`input/pagecontent/test_conversions.md\`, and this section is the
notice of the change that section 4(b) of the Apache License 2.0 asks for,
kept here because a line in the files would break them as messages:

$derived

The page states no Bundle for any of them ("To be provided"), so the corpus
test reads the HL7-authored \`samples/fhir-bundles\` Bundles as the
comparison where one exists.
PROV
  fi
  report "$dest" "$item_v2tofhir"
}

# The build-time NIST bundle of the row $1 into $base/nist/$2: every
# `Context*/**/Message.txt` under `src/main/resources/`, and the bundle's
# `About/Disclaimer.html`.
fetch_nist() {
  local item="$1" dest="$base/nist/$2" pin repo commit top stage="$tmp/nist-$2"
  if [ "$stamp" -eq 0 ] && at_pin "$dest" "$item"; then
    report "$dest" "$item"
    return
  fi
  pin="$(corpus_pin_cell "$item")"
  repo="$(corpus_pin_repo "$pin")"
  commit="$(corpus_pin_commit "$pin")"
  top="${repo##*/}-$commit"
  say "$repo at $commit ($2)"
  mkdir -p "$stage"
  corpus_download "https://codeload.github.com/$repo/tar.gz/$commit" "$tmp/nist.tar.gz" ||
    die "download of $repo at $commit failed"
  tar -tzf "$tmp/nist.tar.gz" |
    grep -E "^$top/src/main/resources/(Context[^/]*/.*/Message\.txt|About/Disclaimer\.html)$" > "$tmp/nist.list" ||
    die "the archive of $repo does not carry commit $commit with its messages"
  tar -xzf "$tmp/nist.tar.gz" -C "$stage" -T "$tmp/nist.list"
  rm -f "$tmp/nist.tar.gz"
  mv "$stage/$top/src" "$stage/src"
  rmdir "$stage/$top"
  verify_stage "$stage" "$item" "$base/nist/PROVENANCE.md"
  install_stage "$stage" "$dest"
  git check-ignore -q "$dest/src" || die "$dest is not ignored by git; the tree must never be committed"
  report "$dest" "$item"
}

# The AIRA MQE example files the corpus reads, each a run of VXU^V04 messages.
aira_files=(
  "Coded_Values_2243_messages.hl7.txt"
  "Fatal_Issues_12_messages.hl7.txt"
  "Large_File_A_2496_messages.hl7.txt"
  "Large_File_B_2548 messages.hl7.txt"
  "Large_File_C_2548_messages.hl7.txt"
  "Large_File_D_1863_messages.hl7.txt"
  "Large_File_E_1859_messages.hl7.txt"
  "NIST_2014_Test_Cases_8_messages.hl7.txt"
  "NIST_2015_Test_Cases_6_messages.hl7.txt"
)

fetch_aira() {
  local dest="$base/aira-mqe" pin repo commit name stage="$tmp/aira"
  if [ "$stamp" -eq 0 ] && at_pin "$dest" "$item_aira"; then
    report "$dest" "$item_aira"
    return
  fi
  pin="$(corpus_pin_cell "$item_aira")"
  repo="$(corpus_pin_repo "$pin")"
  commit="$(corpus_pin_commit "$pin")"
  say "$repo at $commit"
  mkdir -p "$stage/examples"
  for name in "${aira_files[@]}"; do
    corpus_download "https://raw.githubusercontent.com/$repo/$commit/examples/${name// /%20}" "$stage/examples/$name" ||
      die "download of examples/$name from $repo failed"
  done
  verify_stage "$stage" "$item_aira" "$dest/PROVENANCE.md"
  install_stage "$stage" "$dest"
  git check-ignore -q "$dest/examples" || die "$dest is not ignored by git; the tree must never be committed"
  report "$dest" "$item_aira"
}

stamp_build_time() {
  local lri loi ss aira disclaimer lri_state loi_state ss_state aira_state
  lri="$(corpus_pin_cell "$item_lri")"
  loi="$(corpus_pin_cell "$item_loi")"
  ss="$(corpus_pin_cell "$item_ss")"
  aira="$(corpus_pin_cell "$item_aira")"
  lri_state="$(tree_state "$base/nist/lri-r2")"
  loi_state="$(tree_state "$base/nist/loi-r1")"
  ss_state="$(tree_state "$base/nist/ss-r2")"
  aira_state="$(tree_state "$base/aira-mqe")"
  disclaimer="$(sed 's/<[^>]*>//g' "$base/nist/lri-r2/src/main/resources/About/Disclaimer.html" |
    tr -s '[:space:]' ' ' | grep -oE 'This software was developed.*modified\.' || true)"
  [ -n "$disclaimer" ] || die "the LRI bundle's About/Disclaimer.html carries no public-domain statement"
  cat > "$base/nist/PROVENANCE.md" << PROV
<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: the NIST HL7 v2 test bundles

Fetched verbatim at build time by \`scripts/vendor/hl7v2-samples.sh
--build-time\` into this directory, which \`.gitignore\` refuses except for
this file. Never commit a file from here and never edit one: change a pin in
docs/VERSIONS.md, run the script with \`--build-time --stamp\`, and record the
counts and digests it prints.

- Source: <https://github.com/$(corpus_pin_repo "$lri")>, three branches, each
  pinned by commit
- \`lri-r2/\`: branch \`lri-r2\` at commit \`$(corpus_pin_commit "$lri")\`,
  the Laboratory Results Interface bundle: ${lri_state%% *} files, tree
  digest \`${lri_state#* }\`
- \`loi-r1/\`: branch \`loi-r1\` at commit \`$(corpus_pin_commit "$loi")\`,
  the Laboratory Orders Interface bundle: ${loi_state%% *} files, tree
  digest \`${loi_state#* }\`
- \`ss-r2/\`: branch \`ss-r2\` at commit \`$(corpus_pin_commit "$ss")\`, the
  syndromic surveillance bundle: ${ss_state%% *} files, tree digest
  \`${ss_state#* }\`
- Taken from each: every \`src/main/resources/Context*/**/Message.txt\` and
  \`src/main/resources/About/Disclaimer.html\`, at their upstream layout
- Tree digest: sha256 over the sorted per-file \`sha256  path\` listing of each
  branch directory
- Stamped: $(corpus_fetched). The script verifies each count and digest on
  every fetch, and keeps a tree already on disk at its digest.

## Terms

The repository carries no licence file. Each bundle's
\`About/Disclaimer.html\` states, quoted from the LRI bundle with its markup
removed:

> $disclaimer

The test messages are NIST's own test data, never patient data. The corpus
test reads them as message-only smoke coverage and writes nothing from them
into this repository; the owner ruled on 2026-09-25 (#292) to fetch them at
build time.
PROV
  cat > "$base/aira-mqe/PROVENANCE.md" << PROV
<!-- This file describes third-party material fetched at build time; the bytes
     it describes keep their upstream terms and are never committed. -->

# Provenance: the AIRA MQE example messages

Fetched verbatim at build time by \`scripts/vendor/hl7v2-samples.sh
--build-time\` into this directory, which \`.gitignore\` refuses except for
this file. Never commit a file from here and never edit one: change the pin in
docs/VERSIONS.md, run the script with \`--build-time --stamp\`, and record the
count and digest it prints.

- Source: <https://github.com/$(corpus_pin_repo "$aira")>
- Pin: commit \`$(corpus_pin_commit "$aira")\`, the nine \`examples/*.hl7.txt\`
  files, each a run of generated VXU^V04 2.5.1 messages
- Files: ${aira_state%% *}
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`${aira_state#* }\`
- Stamped: $(corpus_fetched)

## Terms

The repository states no licence: it has no licence file at this commit and
GitHub reports none for it. With no grant to redistribute, the files are
fetched at build time and never committed; the owner ruled on 2026-09-25
(#292). The corpus test reads the first messages of each file as a
deterministic smoke subset.
PROV
}

if [ "$scope" = "vendored" ]; then
  vendor_converter
  vendor_reportstream
  vendor_v2tofhir
  say "done"
  exit 0
fi

fetch_nist "$item_lri" lri-r2
fetch_nist "$item_loi" loi-r1
fetch_nist "$item_ss" ss-r2
fetch_aira
if [ "$stamp" -eq 1 ]; then
  stamp_build_time
  say "rewrote $base/nist/PROVENANCE.md and $base/aira-mqe/PROVENANCE.md; record the counts and digests above in docs/VERSIONS.md"
fi
say "done"
