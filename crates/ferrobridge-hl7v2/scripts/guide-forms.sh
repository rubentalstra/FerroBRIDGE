#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# crates/ferrobridge-hl7v2/scripts/guide-forms.sh
#
# Lists the distinct condition and assignment forms the vendored v2-to-FHIR
# ConceptMaps write, with a count and one example each. A form is the text
# with every quoted literal replaced by L, every component or field operand
# (HD.2, HD-3, PID-5.1) by X, and every number by N, so rows that differ only
# in the operands and literals they name fall together.
#
# Usage:
#   crates/ferrobridge-hl7v2/scripts/guide-forms.sh conditions
#   crates/ferrobridge-hl7v2/scripts/guide-forms.sh assignments
#
# Requires: jq, awk, sort.
#
# No specification governs this script; it is FerroBRIDGE's own design.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
package="$root/tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/package"

shape='gsub("\"[^\"]*\""; "L")
  | gsub("[A-Za-z][A-Za-z0-9]{1,3}[-.][0-9]+(\\.[0-9]+)*"; "X")
  | gsub("[0-9]+"; "N")'

case "${1:-}" in
  conditions)
    select='.dependsOn[]? | select(.property == "Computable-ANTLR") | .value'
    ;;
  assignments)
    select='.extension[]? | select(.url | endswith("/TypeInfo"))
      | .extension[] | select(.url == "assignment") | .valueString'
    ;;
  *)
    echo "usage: $0 conditions|assignments" >&2
    exit 64
    ;;
esac

jq -r "
  .group[]?.element[]?.target[]? | $select
  | [($shape), .] | @tsv
" "$package"/ConceptMap-*.json \
  | sort \
  | awk -F '\t' '
      $1 != form {
        if (count) printf "%5d  %s    e.g. %s\n", count, form, example
        form = $1; example = $2; count = 0
      }
      { count++ }
      END { if (count) printf "%5d  %s    e.g. %s\n", count, form, example }
    ' \
  | sort -rn
