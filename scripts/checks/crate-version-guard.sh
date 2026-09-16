#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# The crate bump rule (.claude/rules/crates-publishing.md): a change that alters
# the PACKAGED content of a `crates/*` member (what its `include` ships:
# `src/**`, `README.md`, `LICENSE`, `Cargo.toml`) bumps THAT member's version in
# the same change, because a published version is immutable. A root
# `[workspace.dependencies]` entry a member consumes is packaged content too:
# `cargo package` renders the concrete requirement.
#
# Versions are per crate here, not lockstep: `fhir-types` carries the crate line
# and every other member still holds its name at the 0.0.0 placeholder
# (docs/VERSIONS.md).
#
#   crate-version-guard.sh <base-ref> [head-ref]
#
# The head ref may be the literal WORKTREE, which compares the base with the
# tree as it stands rather than with a commit. That is what the pre-commit hook
# passes: the manifests and Cargo.lock are read from disk either way, so the
# changed set has to come from the same place to agree with them.
#
# Exit 0 when no packaged content changed, or every member whose packaged
# content changed also moved its version, with the root requirement and
# Cargo.lock following. Exit 1 otherwise. The `no-crate-bump` pull-request label
# is the CI escape for a diff that provably does not alter packaged bytes; this
# script does not read labels.
set -euo pipefail
cd "$(dirname "$0")/../.."

base="${1:?usage: crate-version-guard.sh <base-ref> [head-ref|WORKTREE]}"
head="${2:-HEAD}"

# `git diff <base> -- …` with no second ref reads the working tree, which is
# the WORKTREE head; everything else names two commits.
changed_paths() {
  if [[ "$head" = "WORKTREE" ]]; then git diff --name-only "$base" --; else git diff --name-only "$base" "$head" --; fi
}
diff_text() {
  if [[ "$head" = "WORKTREE" ]]; then git diff "$base" -- "$@"; else git diff "$base" "$head" -- "$@"; fi
}
head_file() {
  if [[ "$head" = "WORKTREE" ]]; then cat "$1"; else git show "$head:$1"; fi
}

changed="$(changed_paths)"

# Only the `[workspace.dependencies]` table renders into a packaged manifest;
# the `[workspace.package]` keys (the product `version`, edition, licence) are
# read through `key.workspace = true` and change no crate's packaged bytes.
workspace_dependencies() {
  awk '/^\[workspace\.dependencies\]/{p=1; next} /^\[/{p=0} p && /^[A-Za-z0-9_-]+[[:space:]]*=/{sub(/[[:space:]]*=.*/, ""); print}'
}

# The workspace dependency names this change touched, restricted to names the
# table actually declares on either side.
touched_dependencies=""
if grep -qx 'Cargo.toml' <<<"$changed"; then
  diff_names="$(diff_text Cargo.toml |
    grep -E '^[+-][A-Za-z0-9_-]+[[:space:]]*=' |
    sed -E 's/^[+-]//; s/[[:space:]]*=.*//' | sort -u || true)"
  dependency_names="$( { head_file Cargo.toml; git show "$base:Cargo.toml"; } | workspace_dependencies)"
  for name in $diff_names; do
    grep -qx "$name" <<<"$dependency_names" || continue
    touched_dependencies="$touched_dependencies $name"
  done
fi

package_field() {
  awk -F'"' -v key="$1" '/^\[package\]/{p=1} p && $0 ~ "^" key " = " {print $2; exit}'
}

fail=0
bumped=""
clean=""
reserved=""
for manifest in crates/*/Cargo.toml; do
  crate="$(dirname "$manifest")"
  name="$(package_field name < "$manifest")"

  # A here-string, not a pipe: `grep -q` closes its input on the first match,
  # and under `pipefail` the SIGPIPE'd writer would fail the whole test.
  packaged=0
  # Everything a manifest's `include` can ship: src, the schemas a crate embeds,
  # the README, the licence and the manifest itself.
  if grep -qE "^${crate}/(src/|schemas/|README\.md$|LICENSE$|Cargo\.toml$)" <<<"$changed"; then
    packaged=1
  fi
  if [[ "$packaged" -eq 0 ]]; then
    for dep in $touched_dependencies; do
      if grep -qE "^${dep}(\.workspace)?[[:space:]]*=" "$manifest"; then
        echo "crate-version-guard: $name consumes workspace dependency '$dep', which this change moved."
        packaged=1
        break
      fi
    done
  fi
  if [[ "$packaged" -eq 0 ]]; then
    clean="$clean $name"
    continue
  fi

  new_ver="$(package_field version < "$manifest")"
  old_ver="$(git show "$base:$manifest" 2>/dev/null | package_field version || true)"
  # A 0.0.0 manifest is a crates.io name reservation outside the crate line
  # (docs/VERSIONS.md); its content changes until the first real version.
  if [[ "$new_ver" = "0.0.0" ]] && [[ "${old_ver:-0.0.0}" = "0.0.0" ]]; then
    reserved="$reserved $name"
    continue
  fi
  if [[ -n "$old_ver" ]] && [[ "$old_ver" = "$new_ver" ]]; then
    echo "::error::packaged content of $name changed but its version is still $new_ver. Bump $manifest, move any internal requirement in the root Cargo.toml with it, refresh Cargo.lock, or apply the 'no-crate-bump' label when the diff provably does not alter packaged bytes." >&2
    fail=1
    continue
  fi

  # An internal requirement only exists for a crate other members depend on;
  # when it does, it must name the same version.
  req="$(grep -E "^${name} = \{ path = \"${crate}\", version = \"" Cargo.toml |
    sed -E 's/.*version = "([^"]+)".*/\1/' || true)"
  if [[ -n "$req" ]] && [[ "$req" != "$new_ver" ]]; then
    echo "::error::root Cargo.toml requires $name $req, its manifest is at $new_ver." >&2
    fail=1
  fi
  locked="$(awk -v n="\"$name\"" '$1 == "name" && $3 == n { hit = 1; next } hit && $1 == "version" { gsub(/"/, "", $3); print $3; exit }' Cargo.lock)"
  if [[ "$locked" != "$new_ver" ]]; then
    echo "::error::Cargo.lock records $name ${locked:-nothing}, its manifest is at $new_ver; run cargo update -w and commit the lock." >&2
    fail=1
  fi
  bumped="$bumped $name@${old_ver:-<new>}->$new_ver"
done

[[ "$fail" -eq 0 ]] || exit 1
if [[ -z "$bumped" ]]; then
  echo "crate-version-guard: no packaged content of crates/* changed."
else
  echo "crate-version-guard: packaged content changed and the version moved:$bumped"
fi
[[ -z "$clean" ]] || echo "crate-version-guard: unchanged packaged content:$clean"
[[ -z "$reserved" ]] || echo "crate-version-guard: 0.0.0 name reservations, outside the line:$reserved"
