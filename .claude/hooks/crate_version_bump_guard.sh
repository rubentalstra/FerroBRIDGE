#!/usr/bin/env bash
# SPDX-FileCopyrightText: Ruben Talstra
# SPDX-License-Identifier: BUSL-1.1
# .claude/hooks/crate_version_bump_guard.sh
#
# Claude Code PreToolUse hook (matcher: Bash).
#
# A published crates.io version is immutable, so packaged content that changes
# without a version bump is a defect that cannot be repaired after the upload
# (.claude/rules/crates-publishing.md). CI catches it on the pull request; this
# catches it before the commit exists, which is where the fix is a one-line
# edit rather than a follow-up commit.
#
# Runs scripts/checks/crate-version-guard.sh against the merge base with
# origin/main: WORKTREE for a `git commit`, so the staged change is what is
# checked, and HEAD for a `git push`, where the commits already exist. Exit 2
# blocks the tool call and returns the guard's findings; every other path is a
# quiet exit 0.

set -uo pipefail

payload="$(cat)" || true

if command -v jq > /dev/null 2>&1; then
  command_text="$(printf '%s' "$payload" | jq -r '.tool_input.command // empty' 2>/dev/null)" || true
else
  command_text="$payload"
fi

[ -n "${command_text:-}" ] || exit 0

case "$command_text" in
*"git commit"*) head=WORKTREE ;;
*"git push"*) head=HEAD ;;
*) exit 0 ;;
esac

repo_root="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "$0")/../.." && pwd)}"
guard="$repo_root/scripts/checks/crate-version-guard.sh"
[ -x "$guard" ] || exit 0
[ -d "$repo_root/crates" ] || exit 0

cd "$repo_root" || exit 0

# No origin/main to compare against (a fresh clone with no fetch, a detached
# checkout) means no base, and a guard with no base has nothing to say.
base="$(git merge-base origin/main HEAD 2> /dev/null)" || exit 0
[ -n "$base" ] || exit 0

findings="$(bash "$guard" "$base" "$head" 2>&1)" || {
  printf 'BLOCKED: a crates/* member changed its packaged content without moving its version.\n\n%s\n\n' "$findings" >&2
  printf 'Bump that member in its own Cargo.toml, move any internal requirement in the root Cargo.toml with it, run cargo update -w, and commit the lock. The published version is immutable, so this cannot be repaired later.\n' >&2
  exit 2
}

exit 0
