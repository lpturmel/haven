#!/usr/bin/env bash
# factory/checks.sh — the repo's test, lint, typecheck and build commands.
# The only repo-specific script. factory/bin/gate runs the MAIN checkout's copy
# with the job worktree as the current directory, so this file is judged code,
# never judged by a branch. Exit non-zero on any failure.
#
# Env: FACTORY_WORKTREE (= $PWD), FACTORY_ROOT (main checkout).
set -euo pipefail

step() { printf '\n==> %s\n' "$*"; "$@"; }

# Onboarding replaces the block below with the commands it detected and you
# confirmed. One `step` per command; keep them non-interactive.
step bash -c 'cargo fmt --all -- --check'
step bash -c 'cargo clippy --workspace --all-targets -- -D warnings'
step bash -c 'cargo test --workspace'
