#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# Regenerates, or checks, the sqlx query metadata of omop-cdm (#89). The
# crate's queries are checked at compile time from crates/omop-cdm/.sqlx/, so a
# build needs no database; this script is how that metadata is produced.
#
#   scripts/checks/sqlx-offline.sh           # rewrite crates/omop-cdm/.sqlx/
#   scripts/checks/sqlx-offline.sh --check   # fail when it is stale
#
# It applies the vendored OHDSI DDL, primary keys and indices into a `cdm`
# schema and runs `cargo sqlx prepare` with `search_path` set to that schema,
# the way omop_cdm::database::CdmPool connects. The database is one of two:
#
#   - DATABASE_URL unset: the script starts the PostgreSQL image the
#     docs/VERSIONS.md row pins in Docker and stops it on exit.
#   - DATABASE_URL set (the sqlx-offline CI job, against its service
#     container): the script uses that empty database through `psql`. The URL
#     carries no query string, because the script appends the search_path.
#
# Needs cargo and sqlx-cli at the docs/VERSIONS.md pin, plus Docker or psql.
#
# Exit 0 when the metadata is written or current, 1 when --check finds it
# stale, 2 when a prerequisite is missing or the database does not start.
set -euo pipefail
cd "$(dirname "$0")/../.."

readonly MATRIX=docs/VERSIONS.md
readonly DDL_DIR=crates/omop-cdm/ddl
readonly SCHEMA=cdm
readonly ROLE=ferrobridge

check=false
case "${1:-}" in
  "") ;;
  --check) check=true ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2
    ;;
esac

require() {
  if ! command -v "$1" > /dev/null; then
    echo "error: $1 is not on PATH" >&2
    exit 2
  fi
}

require cargo
if ! cargo sqlx --version > /dev/null 2>&1; then
  echo "error: sqlx-cli is not installed (cargo sqlx)" >&2
  exit 2
fi

if [[ -n "${DATABASE_URL:-}" ]]; then
  require psql
  if [[ "$DATABASE_URL" == *\?* ]]; then
    echo "error: DATABASE_URL carries a query string; pass the bare URL" >&2
    exit 2
  fi
  base_url="$DATABASE_URL"
  sql() {
    psql --quiet --set=ON_ERROR_STOP=1 --dbname="$base_url" "$@"
  }
else
  require docker
  image=$(awk -F'|' '$2 ~ /^ PostgreSQL image / { gsub(/[` ]/, "", $3); print $3 }' "$MATRIX")
  if [[ -z "$image" ]]; then
    echo "error: $MATRIX has no PostgreSQL image row" >&2
    exit 2
  fi

  container=$(docker run --detach --rm \
    --env POSTGRES_USER="$ROLE" --env POSTGRES_PASSWORD="$ROLE" --env POSTGRES_DB="$ROLE" \
    --publish 127.0.0.1::5432 "$image")
  trap 'docker stop "$container" > /dev/null' EXIT

  ready=false
  for _ in $(seq 1 60); do
    if docker exec "$container" pg_isready --username="$ROLE" --dbname="$ROLE" > /dev/null 2>&1; then
      ready=true
      break
    fi
    sleep 1
  done
  if [[ "$ready" != true ]]; then
    echo "error: PostgreSQL did not become ready within 60s" >&2
    exit 2
  fi

  port=$(docker port "$container" 5432/tcp | awk -F: 'NR == 1 { print $NF }')
  base_url="postgres://$ROLE:$ROLE@127.0.0.1:$port/$ROLE"
  sql() {
    docker exec --interactive "$container" \
      psql --quiet --set=ON_ERROR_STOP=1 --username="$ROLE" --dbname="$ROLE" "$@"
  }
fi

sql --command="CREATE SCHEMA $SCHEMA"
for file in ddl primary_keys indices; do
  sed "s/@cdmDatabaseSchema/$SCHEMA/g" "$DDL_DIR/OMOPCDM_postgresql_5.4_$file.sql" | sql
done

export DATABASE_URL="$base_url?options=-c%20search_path%3D$SCHEMA"

cd crates/omop-cdm
if [[ "$check" == true ]]; then
  if ! cargo sqlx prepare --check -- --all-targets --locked; then
    echo "STALE: crates/omop-cdm/.sqlx/ does not match the queries; run $0" >&2
    exit 1
  fi
  echo "crates/omop-cdm/.sqlx/ is current"
else
  cargo sqlx prepare -- --all-targets --locked
fi
