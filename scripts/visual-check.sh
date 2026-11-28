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
#   start.png    start page (pinned fixture folders / drives / recents)
#   list.png     list view   — columns must line up under their headers
#   grid.png     grid view (includes a HEIC photo thumbnail)
#   columns.png  Miller columns
#
# plus one capture per scripts/visual-scenarios/<name>.json (format in
# scripts/visual-scenario.py): a UI feature adds a scenario so its look is
# captured on every later run too. <output-dir>/captures.txt lists every
# capture with its description.
#
# VISUAL_WINDOW_ONLY=1 is for unattended runs (the factory): Haven launches in
# the background with HAVEN_CAPTURE=1 (it never takes focus; its window floats
# above other apps, ignoring clicks, so it keeps redrawing) and only
# <name>.png of the window alone is captured, so nothing else on screen is
# ever recorded. The sidebar luminance check, which needs the composite over
# the desktop, is skipped.
#
# VISUAL_ROOT=<checkout> builds and captures that checkout (its code and its
# scenarios) with this copy of the script and helpers. The factory uses it to
# judge a job's worktree with the main checkout's checker.
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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${VISUAL_ROOT:-${SCRIPT_DIR}/..}" && pwd)"
OUT_DIR="${1:-${ROOT_DIR}/target/visual}"
APP="${ROOT_DIR}/target/bundle/Haven.app"
HELPER="${SCRIPT_DIR}/haven-window.swift"
SCENARIO_TOOL="${SCRIPT_DIR}/visual-scenario.py"
SCENARIO_DIR="${VISUAL_SCENARIOS:-${ROOT_DIR}/scripts/visual-scenarios}"
# WCAG relative luminance of the sidebar background. White text needs <= ~0.18
# for 4.5:1 contrast; keep headroom for the dimmer section labels.
SIDEBAR_MAX_LUMA="${SIDEBAR_MAX_LUMA:-0.12}"
WINDOW_ONLY="${VISUAL_WINDOW_ONLY:-0}"
OPEN_FLAGS=(-n)
# HAVEN_CAPTURE=1: Haven never activates itself and floats its window above
# other apps without taking focus or clicks, so gpui keeps drawing it.
[[ "${WINDOW_ONLY}" == 1 ]] && OPEN_FLAGS=(-n -g --env HAVEN_CAPTURE=1)

EXE="${APP}/Contents/MacOS/haven"
if pgrep -f "${EXE}" >/dev/null; then
    echo "error: this checkout's Haven.app is already running; quit it first:" >&2
    pgrep -fl "${EXE}" >&2
    exit 2
fi

PROFILE="${PROFILE:-debug}" "${ROOT_DIR}/scripts/bundle-macos.sh" >/dev/null
mkdir -p "${OUT_DIR}"
rm -f "${OUT_DIR}"/*.png "${OUT_DIR}/captures.txt"

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

# Pin the fixture's folders on the start page instead of the default
# ~/Desktop, ~/Documents and ~/Downloads. Those are privacy-protected, and the
# first launch of a freshly built (ad-hoc signed) bundle can take seconds to
# stat them while macOS evaluates the new signature, so start.png raced the
# pins and showed the empty placeholder. Fixture pins load at once and keep the
# owner's folders out of the captures.
default_state() {
    printf '{"default_view":"%s","start_pins":["%s/docs","%s/src","%s/assets"],"session":{"tabs":[null],"active":0}}\n' \
        "$1" "${FIXTURE}" "${FIXTURE}" "${FIXTURE}" >"${STATE}/state.json"
}

# capture <name> [KEY=VALUE ...] -- [args ...]   (state.json written first)
capture() {
    local name="$1"
    shift
    local envs=()
    while [[ $# -gt 0 && "$1" != "--" ]]; do
        envs+=(--env "$1")
        shift
    done
    [[ $# -gt 0 ]] && shift
    open "${OPEN_FLAGS[@]}" --env "HAVEN_STATE_DIR=${STATE}" ${envs[@]+"${envs[@]}"} "${APP}" --args "$@"
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
    if [[ "${WINDOW_ONLY}" == 1 ]]; then
        screencapture -x -o -l "${wid}" "${OUT_DIR}/${name}.png"
        quit_ours "${pid}"
        echo "${bounds}"
        return 0
    fi
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
# check <name> <bounds> <check-sidebar 1|0> <description>
check() {
    local name="$1" bounds="$2" check_sidebar="$3" desc="$4"
    printf '%s\t%s\n' "${name}" "${desc}" >>"${OUT_DIR}/captures.txt"
    if [[ "${WINDOW_ONLY}" == 1 ]]; then
        printf '%-8s %s  window only\n' "${name}" "${OUT_DIR}/${name}.png"
        [[ -f "${OUT_DIR}/${name}.png" ]] || { echo "error: no capture for ${name}" >&2; status=1; }
        return 0
    fi
    if [[ ! -f "${OUT_DIR}/${name}.png" ]]; then
        printf '%-8s %s  SKIPPED composite (another window covered Haven); layout only\n' \
            "${name}" "${OUT_DIR}/${name}.window.png"
        return 0
    fi
    if [[ "${check_sidebar}" != 1 ]]; then
        printf '%-8s %s  sidebar check off for this scenario\n' "${name}" "${OUT_DIR}/${name}.png"
        return 0
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
}

for scenario in "start List Start page: pinned fixture folders, drives, recents" \
    "list List List view: Date, Size and Kind under their headers, rows full width" \
    "grid Grid Grid view: icons and the HEIC thumbnail, labels truncated with …" \
    "columns Columns Columns view: Miller columns, rows full width"; do
    read -r name view desc <<<"${scenario}"
    default_state "${view}"
    if [[ "${name}" == start ]]; then
        bounds="$(capture "${name}" --)"
    else
        bounds="$(capture "${name}" -- "${FIXTURE}")"
    fi
    check "${name}" "${bounds}" 1 "${desc}"
done

if [[ -d "${SCENARIO_DIR}" ]]; then
    for file in "${SCENARIO_DIR}"/*.json; do
        [[ -e "${file}" ]] || continue
        launch="${STATE}/launch"
        python3 "${SCENARIO_TOOL}" "${file}" "${FIXTURE}" "${STATE}/state.json" >"${launch}" || exit 2
        fields=()
        while IFS= read -r -d '' f; do fields+=("${f}"); done <"${launch}"
        name="${fields[0]}"
        bounds="$(capture "${name}" "${fields[@]:3}")"
        check "${name}" "${bounds}" "${fields[1]}" "${fields[2]}"
    done
fi

rm -rf "${STATE}"
echo "fixture: ${FIXTURE}"
echo "Review the screenshots: sidebar text legible, card opaque and rounded, traffic"
echo "lights clear of content, list columns aligned under headers, rows full width."
exit "${status}"
