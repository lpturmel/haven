#!/usr/bin/env bash
# factory/visual.sh <out-dir> — tier-3 visual captures for a factory job.
#
# factory/bin/gate runs the MAIN checkout's copy, with the job worktree as the
# current directory, for every job whose surfaces include one listed under
# config.json "visual.surfaces". Like checks.sh, this file and the checker it
# calls come from the main checkout, so a branch can't weaken what judges it;
# the branch supplies the code under test and its scripts/visual-scenarios/.
#
# Exit 0: captures written to <out-dir> (PNGs + captures.txt) and the
# automatic checks passed. Exit 1: an automatic check failed (the builder
# fixes it). Any other exit: the check could not run.
#
# Env: FACTORY_WORKTREE (= $PWD), FACTORY_ROOT (main checkout).
set -euo pipefail

out="${1:?usage: factory/visual.sh <out-dir>}"
# Window-only: unattended, so never take focus or record anything but Haven.
VISUAL_ROOT="${FACTORY_WORKTREE}" VISUAL_WINDOW_ONLY=1 PROFILE=debug \
    bash "${FACTORY_ROOT}/scripts/visual-check.sh" "${out}"
