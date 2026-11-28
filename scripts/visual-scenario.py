#!/usr/bin/env python3
"""visual-scenario: turn one scenario file into a launch for scripts/visual-check.sh.

    scripts/visual-scenario.py <scenario.json> <fixture-dir> <state.json-out>

A scenario file lives in scripts/visual-scenarios/<name>.json and adds one
capture to every visual check, so a UI feature keeps its own regression shot:

    {
      "description": "20 tabs at 900x560: + stays visible, active tab in view",
      "view": "List",                 // default_view (List, Grid, Columns)
      "size": "900x560",              // optional, sets HAVEN_WINDOW_SIZE
      "env": {"HAVEN_FONT_SMOOTHING": "1"},   // optional extra launch env
      "state": {"session": {"tabs": ["$FIXTURE", "$FIXTURE/src"], "active": 1}},
      "args": ["$FIXTURE"],           // optional folder(s) to open
      "check_sidebar": true           // optional; false skips the luminance check
    }

The capture is named after the file. "$FIXTURE" anywhere in a string becomes
the fixture folder. "state" is merged over the default state (fixture start
pins, one tab), key by key at the top level. "tabs" may also be
{"repeat": <n>, "path": "<path or null>"} for n identical tabs.

Writes the state file and prints NUL-separated fields on stdout:
name, check_sidebar (1|0), description, then KEY=VALUE env entries, "--",
then the launch args.
"""
import json
import sys
from pathlib import Path

VIEWS = {"List", "Grid", "Columns"}


def subst(value, fixture):
    if isinstance(value, str):
        return value.replace("$FIXTURE", fixture)
    if isinstance(value, list):
        return [subst(v, fixture) for v in value]
    if isinstance(value, dict):
        return {k: subst(v, fixture) for k, v in value.items()}
    return value


def fail(path, msg):
    sys.stderr.write("error: %s: %s\n" % (path, msg))
    sys.exit(2)


def main(argv):
    if len(argv) != 4:
        sys.stderr.write(__doc__)
        return 2
    path, fixture, state_out = Path(argv[1]), argv[2], Path(argv[3])
    try:
        spec = json.loads(path.read_text())
    except (OSError, ValueError) as e:
        fail(path, "not valid JSON (%s)" % e)
    if not isinstance(spec, dict):
        fail(path, "must be a JSON object")
    name = path.stem
    if not name or not all(c.isalnum() or c in "-_" for c in name):
        fail(path, "file name must be letters, digits, - or _")
    if name in ("start", "list", "grid", "columns"):
        fail(path, "name clashes with a built-in capture")
    view = spec.get("view", "List")
    if view not in VIEWS:
        fail(path, "view must be one of %s" % ", ".join(sorted(VIEWS)))
    spec = subst(spec, fixture)

    state = {
        "default_view": view,
        "start_pins": [fixture + "/docs", fixture + "/src", fixture + "/assets"],
        "session": {"tabs": [None], "active": 0},
    }
    extra = spec.get("state") or {}
    if not isinstance(extra, dict):
        fail(path, '"state" must be an object')
    state.update(extra)
    session = state.get("session") or {}
    tabs = session.get("tabs")
    if isinstance(tabs, dict):
        n = tabs.get("repeat")
        if not isinstance(n, int) or n < 1:
            fail(path, '"tabs": {"repeat": n} needs a positive integer')
        session["tabs"] = [tabs.get("path")] * n
        state["session"] = session
    state_out.write_text(json.dumps(state) + "\n")

    env = dict(spec.get("env") or {})
    size = spec.get("size")
    if size:
        if not isinstance(size, str) or "x" not in size:
            fail(path, '"size" must look like "900x560"')
        env["HAVEN_WINDOW_SIZE"] = size
    args = spec.get("args") or []
    if not isinstance(args, list):
        fail(path, '"args" must be a list')

    fields = [name, "0" if spec.get("check_sidebar") is False else "1",
              " ".join(str(spec.get("description") or "").split())]
    fields += ["%s=%s" % (k, v) for k, v in env.items()]
    fields += ["--"] + [str(a) for a in args]
    sys.stdout.write("".join(f + "\0" for f in fields))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
