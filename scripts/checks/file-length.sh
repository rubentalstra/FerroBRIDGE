#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# File-length guard: a hand-written Rust file is at most $HARD lines, and a
# file over $SOFT lines is split into a module folder before it grows further.
#
# Generated files (a `@generated` banner on the first line) and vendored trees
# are outside the rule. A file listed in scripts/checks/file-length-allow.txt
# is a known breach with the issue that splits it: it passes while it does not
# grow past the line count recorded beside it (the ratchet), and its entry is
# removed by the split. A file over $SOFT and under $HARD that is not listed
# is reported as a warning, never a failure.
#
# Usage:
#   scripts/checks/file-length.sh            check every tracked .rs file
#   scripts/checks/file-length.sh --files F…  check the named files
#   scripts/checks/file-length.sh --record    rewrite the allow-list from the
#                                            current breaches (a split removes
#                                            entries; --record never adds a
#                                            new one without an issue number
#                                            given as --issue N)
# Exit 1 on a hard breach or a listed file that grew; 0 otherwise.
set -euo pipefail
cd "$(dirname "$0")/../.."

SOFT=750
HARD=1000
ALLOW=scripts/checks/file-length-allow.txt

note() { printf '  %s\n' "$*"; }

mode=check
issue=""
files=()
while [ $# -gt 0 ]; do
  case "$1" in
    --files) mode=files; shift; while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do files+=("$1"); shift; done ;;
    --record) mode=record; shift ;;
    --issue) issue="$2"; shift 2 ;;
    *) printf 'file-length: unknown argument %s\n' "$1" >&2; exit 2 ;;
  esac
done

if [ "$mode" != files ]; then
  while IFS= read -r f; do files+=("$f"); done < <(git ls-files -- '*.rs' ':(glob,exclude)**/vendor/**')
fi

# A template under a generator's src/templates/ is the text of one generated
# file, embedded verbatim, so it is generated output in source form.
hand_written() {
  case "$1" in */vendor/*|*/src/templates/*) return 1 ;; esac
  ! head -n1 "$1" | grep -q '@generated'
}

# The allow-list entry of a path, as "<lines> <tag>", or nothing (bash 3.2 has
# no associative arrays, so the list is read per lookup).
listed() {
  [ -f "$ALLOW" ] || return 0
  awk -v p="$1" '$1 !~ /^#/ && $2 == p { print $1, $3 }' "$ALLOW"
}

fail=0
warn=0
breaches=()
# A listed path that no longer exists is a stale entry: the split removed the
# file, so the entry goes with it.
if [ "$mode" = check ] && [ -f "$ALLOW" ]; then
  while IFS=' ' read -r first path _; do
    case "$first" in ''|\#*) continue ;; esac
    [ -f "$path" ] && continue
    note "FAIL $path is listed but does not exist; remove its allow-list entry"
    fail=1
  done < "$ALLOW"
fi
for f in "${files[@]}"; do
  [ -f "$f" ] || continue
  hand_written "$f" || continue
  n=$(wc -l < "$f" | tr -d " ")
  if [ "$n" -gt "$SOFT" ]; then
    breaches+=("$n $f")
  fi
  entry=$(listed "$f")
  if [ -n "$entry" ]; then
    recorded=${entry%% *}
    tag=${entry#* }
    if [ "$n" -gt "$recorded" ]; then
      note "FAIL $f: $n lines, listed at $recorded ($tag); a listed file may not grow, split it"
      fail=1
    elif [ "$n" -le "$SOFT" ]; then
      note "FAIL $f: $n lines, under the limit; remove its allow-list entry"
      fail=1
    fi
    continue
  fi
  if [ "$n" -gt "$HARD" ]; then
    note "FAIL $f: $n lines, over the hard limit of $HARD; split it into a module folder"
    fail=1
  elif [ "$n" -gt "$SOFT" ]; then
    note "warn $f: $n lines, over $SOFT; split it before it grows"
    warn=$((warn + 1))
  fi
done

if [ "$mode" = record ]; then
  {
    printf '# Known file-length breaches: "<lines> <path> #<issue>". A listed file passes\n'
    printf '# while it does not grow past its recorded count; the split removes the entry.\n'
    for b in "${breaches[@]}"; do
      n=${b%% *}; p=${b#* }
      entry=$(listed "$p")
      if [ -n "$entry" ]; then
        printf '%s %s %s\n' "$n" "$p" "${entry#* }"
      elif [ -n "$issue" ]; then
        printf '%s %s #%s\n' "$n" "$p" "$issue"
      else
        printf 'file-length: %s is a new breach; pass --issue N to record it\n' "$p" >&2
        exit 2
      fi
    done | sort -k2
  } > "$ALLOW.tmp" && mv "$ALLOW.tmp" "$ALLOW"
  note "recorded ${#breaches[@]} breaches in $ALLOW"
  exit 0
fi

if [ "$fail" -eq 0 ]; then
  note "OK: no hand-written .rs file over $HARD lines outside the allow-list ($warn over $SOFT unlisted)"
fi
exit "$fail"
