#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/vendor/omop-cdm.sh
#
# Vendors the OMOP CDM v5.4 machine-readable definitions into docs/specs/omop-cdm/
# (.claude/rules/vendored-inputs.md): the CSV table and field definitions the
# `omop-cdm` row types are generated from, the four rendered PostgreSQL DDL files
# a test compares that generated column set against, `site/sqlScripts.qmd`, the
# source of the CDM's SQL scripts page whose era scripts the derived tables
# follow (docs/architecture.md section 5.4), and `DESCRIPTION`, the only place
# the repository declares its licence (docs/architecture.md section 10).
#
# The "OMOP CDM definitions and PostgreSQL DDL" row of docs/VERSIONS.md pins a
# tag. A tag is mutable, so the script resolves it to a commit, fetches that
# commit, and records the commit in the provenance beside the tag.
#
# Usage:
#   scripts/vendor/omop-cdm.sh
#
# Requires: curl, tar, shasum, jq.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# shellcheck source=scripts/vendor/lib/corpus.sh
# shellcheck disable=SC1091 # shellcheck is not run with -x; the library is checked on its own
. "$root/scripts/vendor/lib/corpus.sh"

corpus_require curl tar shasum jq

dest="docs/specs/omop-cdm"
ddl="inst/ddl/5.4/postgresql"

paths=(
  "inst/csv/OMOP_CDMv5.4_Field_Level.csv"
  "inst/csv/OMOP_CDMv5.4_Table_Level.csv"
  "$ddl/OMOPCDM_postgresql_5.4_ddl.sql"
  "$ddl/OMOPCDM_postgresql_5.4_primary_keys.sql"
  "$ddl/OMOPCDM_postgresql_5.4_indices.sql"
  "$ddl/OMOPCDM_postgresql_5.4_constraints.sql"
  "site/sqlScripts.qmd"
  "DESCRIPTION"
)

pin="$(corpus_pin_cell "OMOP CDM definitions and PostgreSQL DDL")"
repo="$(corpus_pin_repo "$pin")"
tag="$(corpus_pin_tag "$pin")"
commit="$(corpus_resolve_tag "$repo" "$tag")"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "$repo tag $tag resolves to $commit"
tree_root="$(corpus_fetch "$repo" "$commit" "$tmp")"

# The repository declares its licence in DESCRIPTION and ships no LICENSE file,
# so the provenance quotes the DESCRIPTION line rather than assuming a licence.
[ ! -e "$tree_root/LICENSE" ] ||
  die "the archive now has a LICENSE file; vendor it and correct the provenance"
licence="$(sed -nE 's/^License:[[:space:]]*//p' "$tree_root/DESCRIPTION" | head -n1)"
[ -n "$licence" ] || die "DESCRIPTION declares no License"

rm -rf "$dest"
mkdir -p "$dest"
corpus_take "$tree_root" "$dest" "${paths[@]}"

rows=""
for path in "${paths[@]}"; do
  rows="$rows
| \`$path\` | \`$(corpus_sha256 "$dest/$path")\` | \`$(corpus_blob_id "$dest/$path")\` |"
done

files="$(corpus_file_count "$dest")"
digest="$(corpus_tree_digest "$dest")"
fetched="$(corpus_fetched)"

cat > "$dest/PROVENANCE.md" << PROV
<!-- This file describes vendored third-party material; the bytes beside it
     keep their upstream licence, not the licence of this repository. -->

# Provenance: the OMOP Common Data Model v5.4 definitions, PostgreSQL DDL and SQL scripts

Vendored verbatim by \`scripts/vendor/omop-cdm.sh\`
(.claude/rules/vendored-inputs.md). Never edit a file here: change the pin in
docs/VERSIONS.md and re-run the script.

- Source: <https://github.com/$repo>
- Pin: tag \`$tag\`, which resolves to commit \`$commit\`
- Fetched: $fetched
- Upstream licence: \`$licence\`. **The repository has no \`LICENSE\` file.**
  \`DESCRIPTION\` is the only place the licence is declared, which is why that
  file is vendored here and the script fails if a \`LICENSE\` file ever appears.
- Layout: the upstream paths, unchanged
- Files: $files
- Tree digest (sha256 over the sorted per-file \`sha256  path\` listing,
  \`PROVENANCE.md\` excluded): \`$digest\`

## What is here

The two CSV files are the machine-readable definitions the \`omop-cdm\` row
types are generated from. The four PostgreSQL files are OHDSI's rendered DDL:
OHDSI renders them from the same CSVs through a dialect layer that sits outside
them, so they are vendored rather than generated, and a test asserts the
generated column set equals the DDL's (docs/architecture.md section 10).

\`site/sqlScripts.qmd\` is the source of the CDM's SQL scripts page
(<https://ohdsi.github.io/CommonDataModel/sqlScripts.html>). Its condition era
and drug era scripts are OHDSI SQL for SqlRender, so the \`omop-cdm\` crate
carries their PostgreSQL form under \`sql/\`, and a test pins the digest of
this file so an upstream change to the scripts is noticed.

| File | sha256 | git blob id |
|---|---|---|$rows
PROV

say "$files files, tree digest $digest"
say "done"
