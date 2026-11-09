#!/usr/bin/env bash
set -euo pipefail
binary=${1:?path to quakewatch-gui required}
export LIBGL_ALWAYS_SOFTWARE=1
export GALLIUM_DRIVER=llvmpipe
"$binary" --demo &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT
wid=''
for i in {1..60}; do
    mapfile -t wins < <(xdotool search --onlyvisible --pid "$pid" --name Quakewatch || true)
    if ((${#wins[@]})); then wid=${wins[0]}; break; fi
    kill -0 "$pid"
    sleep .25
done
[[ -n "$wid" ]]
xwininfo -id "$wid" | grep 'IsViewable'
xdotool windowfocus --sync "$wid"
xdotool key d
sleep .4
xdotool key d
sleep .4
kill -0 "$pid"
