#!/usr/bin/env bash
# Tier-3 visual validation helper.
#
#   scripts/visual-check.sh [output-dir]           # default: target/visual
#   PROFILE=release scripts/visual-check.sh
#
# Builds a fresh Haven.app, launches it on a fixture folder with an isolated
# state dir, and captures the on-screen window (so the frosted sidebar is shown
# composited over the real desktop) for each scenario:
#
#   start.png    start page (pinned / drives / recents)
#   list.png     list view   — columns must line up under their headers
#   grid.png     grid view (includes a HEIC photo thumbnail)
#   columns.png  Miller columns
#
# Each scenario also gets <name>.window.png, the window alone (never covered
# by other apps, but without the desktop behind the blur). If another app's
# window covers Haven when capturing, the composited capture is retried and
# finally skipped (and the legibility check with it) rather than recording
# someone else's screen.
#
# Automatic check: the sidebar's background luminance must stay low enough for
# its white text to be legible over any wallpaper (see SIDEBAR_MAX_LUMA).
# Everything interactive (path bar, menus, tags, undo…) still needs a person or
# an agent with screen control; see the checklist in AGENTS.md.
#
# Safe to run next to other Haven instances (an installed /Applications copy,
# builds from other worktrees): every build shares the bundle id
# `dev.haven.app`, so this script never addresses an app by bundle id. It finds
# its instance by this checkout's bundle path and pid, captures that pid's
# window, and quits it by pid. It only refuses to run if this checkout's own
# bundle is already running (a leftover from an earlier run).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-${ROOT_DIR}/target/visual}"
APP="${ROOT_DIR}/target/bundle/Haven.app"
HELPER="${ROOT_DIR}/scripts/haven-window.swift"
# WCAG relative luminance of the sidebar background. White text needs <= ~0.18
# for 4.5:1 contrast; keep headroom for the dimmer section labels.
SIDEBAR_MAX_LUMA="${SIDEBAR_MAX_LUMA:-0.12}"

EXE="${APP}/Contents/MacOS/haven"
if pgrep -f "${EXE}" >/dev/null; then
    echo "error: this checkout's Haven.app is already running; quit it first:" >&2
    pgrep -fl "${EXE}" >&2
    exit 2
fi

PROFILE="${PROFILE:-debug}" "${ROOT_DIR}/scripts/bundle-macos.sh" >/dev/null
mkdir -p "${OUT_DIR}"

FIXTURE="$(mktemp -d /tmp/haven-visual.XXXXXX)"
mkdir -p "${FIXTURE}/src" "${FIXTURE}/docs" "${FIXTURE}/assets"
cp "${ROOT_DIR}/crates/haven-core/src/sort.rs" "${FIXTURE}/src/sort.rs"
printf '# Notes\n\nhello\n' >"${FIXTURE}/docs/notes.md"
head -c 50000 /dev/urandom >"${FIXTURE}/big.bin"
echo small >"${FIXTURE}/a.txt"
echo x >"${FIXTURE}/a-file-with-a-rather-long-name-to-test-truncation.txt"
touch -t 202001010000 "${FIXTURE}/a.txt"
# A HEIC photo (macOS ships some as wallpapers) exercises native thumbnails.
HEIC="$(find "/System/Library/Desktop Pictures" -maxdepth 1 -name '*.heic' 2>/dev/null | head -1)"
[[ -n "${HEIC}" ]] && cp "${HEIC}" "${FIXTURE}/photo.heic"

STATE="$(mktemp -d /tmp/haven-visual-state.XXXXXX)"
# Stop only the instance(s) launched from this checkout's bundle, never another
# Haven (quitting by bundle id would hit whichever instance came first).
quit_ours() {
    local pid="$1"
    kill -TERM "${pid}" 2>/dev/null || return 0
    for _ in $(seq 1 50); do
        kill -0 "${pid}" 2>/dev/null || return 0
        sleep 0.1
    done
    kill -KILL "${pid}" 2>/dev/null || true
}
trap 'for p in $(pgrep -f "${EXE}"); do quit_ours "${p}"; done' EXIT

capture() {
    local name="$1" view="$2"
    shift 2
    printf '{"default_view":"%s","session":{"tabs":[null],"active":0}}\n' "${view}" \
        >"${STATE}/state.json"
    open -n --env "HAVEN_STATE_DIR=${STATE}" "${APP}" --args "$@"
    local pid=""
    for _ in $(seq 1 50); do
        pid="$(pgrep -n -f "${EXE}" || true)"
        [[ -n "${pid}" ]] && break
        sleep 0.1
    done
    [[ -n "${pid}" ]] || { echo "error: Haven did not start" >&2; exit 1; }
    sleep 2.5 # first paint, icons, blur
    local info wid bounds
    info="$(swift "${HELPER}" bounds "${pid}")"
    wid="${info%% *}"
    bounds="${info#* }"
    screencapture -x -o -l "${wid}" "${OUT_DIR}/${name}.window.png"
    rm -f "${OUT_DIR}/${name}.png"
    for _ in 1 2 3; do
        if ! swift "${HELPER}" occluded "${wid}"; then
            screencapture -x -o -R "${bounds}" "${OUT_DIR}/${name}.png"
            break
        fi
        # Bring it back to the front and try again. LaunchServices resolves
        # the path to this bundle's instance, not another dev.haven.app.
        open -a "${APP}"
        sleep 1
    done
    quit_ours "${pid}"
    echo "${bounds}"
}

status=0
for scenario in "start List" "list List ${FIXTURE}" "grid Grid ${FIXTURE}" "columns Columns ${FIXTURE}"; do
    # shellcheck disable=SC2086
    set -- ${scenario}
    name="$1"
    bounds="$(capture "$@")"
    if [[ ! -f "${OUT_DIR}/${name}.png" ]]; then
        printf '%-8s %s  SKIPPED composite (another window covered Haven); layout only\n' \
            "${name}" "${OUT_DIR}/${name}.window.png"
        continue
    fi
    w="$(cut -d, -f3 <<<"${bounds}")"
    h="$(cut -d, -f4 <<<"${bounds}")"
    # Sample the sidebar's left padding (no text) below the traffic lights.
    fx="$(awk -v w="${w}" 'BEGIN { printf "%.4f", 2 / w }')"
    fw="$(awk -v w="${w}" 'BEGIN { printf "%.4f", 6 / w }')"
    fy="$(awk -v h="${h}" 'BEGIN { printf "%.4f", 60 / h }')"
    fh="$(awk -v h="${h}" 'BEGIN { printf "%.4f", (h - 100) / h }')"
    luma="$(swift "${HELPER}" luma "${OUT_DIR}/${name}.png" "${fx}" "${fy}" "${fw}" "${fh}")"
    verdict="ok"
    if awk -v l="${luma}" -v m="${SIDEBAR_MAX_LUMA}" 'BEGIN { exit !(l > m) }'; then
        verdict="FAIL (sidebar too bright for its text; max ${SIDEBAR_MAX_LUMA})"
        status=1
    fi
    printf '%-8s %s  sidebar luminance %s  %s\n' "${name}" "${OUT_DIR}/${name}.png" "${luma}" "${verdict}"
done

rm -rf "${STATE}"
echo "fixture: ${FIXTURE}"
echo "Review the screenshots: sidebar text legible, card opaque and rounded, traffic"
echo "lights clear of content, list columns aligned under headers, rows full width."
exit "${status}"
