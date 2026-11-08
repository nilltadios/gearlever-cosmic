#!/usr/bin/env bash
# Compare resource use of Gear Lever COSMIC (the AppImage) and Gear Lever
# (Flatpak it.mijorus.gearlever) on the same disposable home folder.
# For each app and run: start time (launch -> window listed by the
# compositor), memory of the whole process tree after 5 s (RSS and PSS from
# /proc/PID/smaps_rollup, so shared libraries are not double counted), and
# idle CPU over 10 s. Also size on disk. Only windows this script opens are
# closed. Needs a Wayland session and lcu (linux-computer-use).
# Usage: verification/resource-usage.sh [APPIMAGE] [RUNS]
# Output: verification/out-resources-*/results.json and a table on stdout.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
appimage="$(realpath "${1:-dist/GearLever_COSMIC-0.1.0-x86_64.AppImage}")"
runs="${2:-3}"
L="${LCU:-$HOME/.claude/plugins/cache/linux-computer-use/linux-computer-use/8f2aaf9869d5/lcu}"
out="$PWD/verification/out-resources-$(date +%Y%m%d-%H%M%S)"
cargo build -q --release --example fixture_runtime
cargo run -q --release --example make_fixture -- target/release/examples/fixture_runtime "$out/fx" 1 >/dev/null
H="$out/fx/home"
mkdir -p "$H/.config/cosmic"
cp -r "$HOME"/.config/cosmic/com.system76.CosmicTheme.* "$HOME"/.config/cosmic/com.system76.CosmicTk "$H/.config/cosmic/" 2>/dev/null || true
GLENV=(--env=HOME="$H" --env=XDG_CONFIG_HOME="$H/.var/gl/config" --env=XDG_DATA_HOME="$H/.var/gl/data" --env=XDG_CACHE_HOME="$H/.var/gl/cache")
# Same three apps for both: integrated by Gear Lever, read by both.
for f in Demo_Editor-1.0-x86_64.AppImage Old_Tool-0.9.AppImage Sky-Notes-2.0.0.AppImage; do
  flatpak run "${GLENV[@]}" it.mijorus.gearlever --integrate "$H/Downloads/$f" -y >/dev/null 2>&1 || true
done
ls "$H/.local/share/applications" > "$out/apps.txt"

tree_pids() { # all descendants of $1, including $1
  local p=$1; echo "$p"
  for c in $(ps -o pid= --ppid "$p" 2>/dev/null); do tree_pids "$c"; done
}
mem_kb() { # "RSS PSS" summed over the tree
  local rss=0 pss=0 r s
  for p in $(tree_pids "$1"); do
    r=$(awk '/^Rss:/{print $2}' /proc/$p/smaps_rollup 2>/dev/null || echo 0)
    s=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup 2>/dev/null || echo 0)
    rss=$((rss + ${r:-0})); pss=$((pss + ${s:-0}))
  done
  echo "$rss $pss"
}
ticks() { # utime+stime summed over the tree, in clock ticks
  local t=0 v
  for p in $(tree_pids "$1"); do
    v=$(awk '{print $14+$15}' /proc/$p/stat 2>/dev/null || echo 0)
    t=$((t + ${v:-0}))
  done
  echo $t
}
wait_window() { # class -> seconds until listed (max 60)
  local start=$1 class=$2
  for _ in $(seq 1 600); do
    if "$L" window list 2>/dev/null | python3 -c "import json,sys; sys.exit(0 if any(w['class']==sys.argv[1] for w in json.load(sys.stdin)) else 1)" "$class"; then
      python3 -c "import time,sys; print(f'{time.time()-float(sys.argv[1]):.2f}')" "$start"; return 0
    fi
    sleep 0.1
  done
  echo "timeout"; return 1
}
hz=$(getconf CLK_TCK)
results="$out/results.jsonl"
for app in cosmic gearlever; do
  for run in $(seq 1 "$runs"); do
    start=$(python3 -c 'import time; print(time.time())')
    if [[ $app == cosmic ]]; then
      inst="dev.nilltadios.GearLeverCosmic.UI.resources$run$$"
      setsid env HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" GEARLEVER_COSMIC_UI_ISOLATED=1 GEARLEVER_COSMIC_UI_INSTANCE="$inst" "$appimage" >"$out/$app-$run.log" 2>&1 &
      class=dev.nilltadios.GearLeverCosmic
    else
      setsid flatpak run "${GLENV[@]}" it.mijorus.gearlever >"$out/$app-$run.log" 2>&1 &
      class=it.mijorus.gearlever
    fi
    pid=$!
    t_window=$(wait_window "$start" "$class" || true)
    sleep 5
    read -r rss pss < <(mem_kb "$pid")
    nproc=$(tree_pids "$pid" | wc -l)
    t0=$(ticks "$pid"); sleep 10; t1=$(ticks "$pid")
    cpu=$(python3 -c "print(round(($t1-$t0)/$hz/10*100, 2))")
    echo "{\"app\":\"$app\",\"run\":$run,\"window_s\":\"$t_window\",\"rss_mb\":$((rss/1024)),\"pss_mb\":$((pss/1024)),\"processes\":$nproc,\"idle_cpu_pct\":$cpu}" | tee -a "$results"
    # Close only the window this run opened.
    if [[ $app == cosmic ]]; then
      HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" "$appimage" cli ui --instance "$inst" close --apply >/dev/null 2>&1 || kill "$pid"
    else
      "$L" window close "class:$class" >/dev/null 2>&1 || true
    fi
    for _ in $(seq 1 50); do kill -0 "$pid" 2>/dev/null || break; sleep 0.2; done
    kill -0 "$pid" 2>/dev/null && { kill "$pid" 2>/dev/null || true; }
    sleep 2
  done
done
gl_size=$(flatpak info it.mijorus.gearlever | sed -n 's/^ *Installed Size: //p')
gl_runtime=$(flatpak info "$(flatpak info -m it.mijorus.gearlever | sed -n 's/^runtime=//p')" 2>/dev/null | sed -n 's/^ *Installed Size: //p')
ours_size=$(du -h "$appimage" | cut -f1)
python3 - "$results" "$gl_size" "$gl_runtime" "$ours_size" "$out" <<'PY'
import json, statistics, sys
rows = [json.loads(l) for l in open(sys.argv[1])]
summary = {}
for app in ("cosmic", "gearlever"):
    r = [x for x in rows if x["app"] == app]
    med = lambda k: statistics.median(float(x[k]) for x in r if x[k] not in ("timeout", ""))
    summary[app] = {k: med(k) for k in ("window_s", "rss_mb", "pss_mb", "idle_cpu_pct", "processes")}
summary["disk"] = {"gearlever_app": sys.argv[2], "gearlever_runtime_org.gnome.Platform": sys.argv[3], "cosmic_appimage": sys.argv[4]}
json.dump({"runs": rows, "median": summary}, open(f"{sys.argv[5]}/results.json", "w"), indent=2)
c, g = summary["cosmic"], summary["gearlever"]
print(f"\n{'median of runs':28} {'Gear Lever COSMIC':>18} {'Gear Lever':>12}")
for k, label in [("window_s", "start to window (s)"), ("pss_mb", "memory PSS (MB)"), ("rss_mb", "memory RSS (MB)"), ("idle_cpu_pct", "idle CPU (% of a core)"), ("processes", "processes")]:
    print(f"{label:28} {c[k]:>18} {g[k]:>12}")
print(f"{'size on disk':28} {sys.argv[4]:>18} {sys.argv[2]:>12} (+ GNOME runtime {sys.argv[3]})")
print(f"\nresults: {sys.argv[5]}/results.json")
PY

# CLI: wall time and peak memory of listing the installed apps (3 runs each).
# Peak memory comes from wait4(): the largest process of the run's tree.
python3 - "$appimage" "$H" "$out" <<'PY'
import json, os, statistics, subprocess, sys, time
appimage, H, out = sys.argv[1:]
env = dict(os.environ, HOME=H, XDG_CONFIG_HOME=f"{H}/.config", XDG_DATA_HOME=f"{H}/.local/share")
glenv = [f"--env=HOME={H}", f"--env=XDG_CONFIG_HOME={H}/.var/gl/config", f"--env=XDG_DATA_HOME={H}/.var/gl/data", f"--env=XDG_CACHE_HOME={H}/.var/gl/cache"]
cmds = {
    "cosmic": [appimage, "cli", "--list-installed", "--json"],
    "gearlever": ["flatpak", "run", *glenv, "it.mijorus.gearlever", "--list-installed", "--json"],
}
res = {}
for app, cmd in cmds.items():
    times, peaks, listed = [], [], None
    for _ in range(3):
        t = time.time()
        # Gear Lever gets the fixture through --env only: flatpak itself must
        # keep the real HOME to find its own installation.
        proc = subprocess.Popen(cmd, env=env if app == "cosmic" else None, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        data = proc.stdout.read()
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
        times.append(time.time() - t)
        peaks.append(usage.ru_maxrss / 1024)
        listed = len(json.loads(data)["installed"])
    res[app] = {"median_s": round(statistics.median(times), 2), "peak_rss_mb": round(statistics.median(peaks)), "apps_listed": listed}
json.dump(res, open(f"{out}/cli.json", "w"), indent=2)
print(f"\n{'CLI --list-installed --json':30} {'Gear Lever COSMIC':>18} {'Gear Lever':>12}")
print(f"{'time (s)':30} {res['cosmic']['median_s']:>18} {res['gearlever']['median_s']:>12}")
print(f"{'peak RSS, largest process (MB)':30} {res['cosmic']['peak_rss_mb']:>18} {res['gearlever']['peak_rss_mb']:>12}")
print(f"{'apps listed':30} {res['cosmic']['apps_listed']:>18} {res['gearlever']['apps_listed']:>12}")
PY
