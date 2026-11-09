#!/usr/bin/env bash
# X11 settings smoke: real UI key handling, persistence, cancel, hook test.
set -euo pipefail
binary=${1:?path to quakewatch-gui required}
export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
base=$(mktemp -d "${TMPDIR:-${RUNNER_TEMP:-.}}/quakewatch-ui.XXXXXX")
export XDG_CONFIG_HOME="$base/config"
mkdir -p "$XDG_CONFIG_HOME/quakewatch"
export QW_UI_DIR="$base"
python - <<'PY'
import json, os
from pathlib import Path
p = Path(os.environ['QW_UI_DIR'])
script = p / 'hook test.sh'
script.write_text('#!/bin/sh\n[ "$#" -eq 2 ] && [ "$1" = 4 ] && [ "$2" = 10 ] && printf ok > "$QW_UI_DIR/result"\n')
script.chmod(0o700)
cfg = dict(enabled=False,program=str(script.resolve()),once=True,scale=100,place=0,threshold=2)
(Path(os.environ['XDG_CONFIG_HOME']) / 'quakewatch/desktop.json').write_text(json.dumps(cfg))
PY
"$binary" >"$base/app.log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; rm -rf "$base"' EXIT
wid=''
for i in {1..120}; do
    mapfile -t wins < <(xdotool search --onlyvisible --pid "$pid" --name Quakewatch || true)
    if ((${#wins[@]})); then wid=${wins[0]}; break; fi
    kill -0 "$pid"
    sleep .25
done
[[ -n "$wid" ]]
timeout 5s xdotool windowfocus --sync "$wid"
wait_title() {
    local title
    for i in {1..80}; do
        title=$(xdotool getwindowname "$wid")
        if [[ "$title" == *"$1"* ]]; then return 0; fi
        kill -0 "$pid"
        sleep .25
    done
    printf 'Expected window title to contain %s, got %s\n' "$1" "$title" >&2
    return 1
}
wait_title '未連線'
xdotool key --clearmodifiers s
wait_title '設定 · 其他'
xdotool key --clearmodifiers t
for i in {1..80}; do
    [[ ! -f "$base/result" ]] || break
    sleep .25
done
[[ "$(<"$base/result")" == ok ]]
sleep .5
xdotool key --clearmodifiers 1
wait_title '設定 · 強震即時警報'
xdotool key --clearmodifiers Return
wait_title '未連線'
python - <<'PY'
import json,os
from pathlib import Path
c=json.loads((Path(os.environ['XDG_CONFIG_HOME'])/'quakewatch/desktop.json').read_text())
assert c['scale']==100 and c['threshold']==2 and c['once'] and not c['enabled']
print('settings saved; hook received exact positional arguments 4 10')
PY
before=$(sha256sum "$XDG_CONFIG_HOME/quakewatch/desktop.json")
xdotool key --clearmodifiers s
wait_title '設定 · 其他'
xdotool key --clearmodifiers Escape
wait_title '未連線'
[[ "$(sha256sum "$XDG_CONFIG_HOME/quakewatch/desktop.json")" == "$before" ]]
printf 'Settings navigation, hook test, save, cancel: OK\n'
